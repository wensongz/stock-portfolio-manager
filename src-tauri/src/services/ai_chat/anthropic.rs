use super::*;

/// Stream an Anthropic Messages API turn. Mirrors `chat_stream`'s agentic
/// tool-calling loop but with the native `/v1/messages` protocol: `x-api-key`
/// auth, `system` as a top-level field, `content` as typed blocks, and SSE
/// events (`content_block_start` / `content_block_delta` / `message_stop`).
pub(super) async fn chat_stream_anthropic(
    app: AppHandle,
    db: &Database,
    cache: &ExchangeRateCache,
    quote_cache: &QuoteCache,
    quote_state: &QuoteServiceState,
    params: ChatParams,
) -> Result<(), String> {
    let emit_error = |app: &AppHandle, msg: String| {
        let _ = app.emit("ai-chat-error", msg);
    };

    let cfg = load_and_validate_config(db)?;
    let base = match resolve_base_url(&cfg.provider, cfg.base_url.as_deref()) {
        Ok(b) => b,
        Err(e) => return Err(e),
    };
    // Native Messages endpoint; user-provided base may already include /v1.
    let url = if base.ends_with("/v1") {
        format!("{base}/messages")
    } else {
        format!("{base}/v1/messages")
    };

    // Resolve skills (same as the OpenAI path).
    let activated = resolve_active_skills(&app, &params);
    if !activated.is_empty() {
        let names: Vec<String> = activated.iter().map(|s| s.name.clone()).collect();
        let _ = app.emit("ai-chat-skill", names);
    }

    // ── Build the conversation as Anthropic messages ────────────────────────
    // system is a top-level field; user/assistant content is a list of text
    // blocks. Tool results are appended as user messages with tool_result
    // blocks during the loop.
    let mut system_parts: Vec<String> = Vec::new();
    if !cfg.system_prompt.trim().is_empty() {
        let prompt = if cfg.tools_enabled {
            cfg.system_prompt.clone()
        } else {
            strip_tools_from_prompt(&cfg.system_prompt)
        };
        system_parts.push(prompt);
    }
    let skill_block = build_skill_system_message(&activated);
    if !skill_block.is_empty() {
        system_parts.push(skill_block);
    }
    if params.include_context {
        match build_portfolio_context(db, cache, quote_cache).await {
            Ok(ctx) => system_parts.push(format!(
                "以下是用户的实时投资组合数据，请在回答时参考（金额单位均为 USD，数据可能略有延迟）：\n\n{ctx}"
            )),
            Err(e) => warn!(target: "ai_chat", "failed to build portfolio context: {e}"),
        }
    }

    // messages: Vec<Value> where each is {role, content: [...]} (or a
    // tool_result user message). Mutated across rounds.
    let mut messages: Vec<serde_json::Value> = Vec::new();
    for m in &params.messages {
        messages.push(json!({
            "role": m.role,
            "content": [ { "type": "text", "text": m.content } ],
        }));
    }

    let tools = if cfg.tools_enabled {
        anthropic_tool_definitions(&crate::services::ai_tools::tool_definitions())
    } else {
        Vec::new()
    };

    let tool_ctx = crate::services::ai_tools::ToolCtx::for_untrusted_model_turn(
        db,
        cache,
        quote_cache,
        quote_state,
        latest_user_message(&params),
    );
    if let Some(context) = &params.tool_context {
        let parsed_arguments = context.arguments.clone();
        let (_, content, _) = execute_prefilled_tool(&app, &tool_ctx, context).await?;
        messages.push(json!({
            "role": "assistant",
            "content": [{
                "type": "tool_use",
                "id": HOST_PREFILLED_TOOL_CALL_ID,
                "name": context.name,
                "input": parsed_arguments,
            }],
        }));
        messages.push(json!({
            "role": "user",
            "content": [{
                "type": "tool_result",
                "tool_use_id": HOST_PREFILLED_TOOL_CALL_ID,
                "content": content,
            }],
        }));
    }

    STOP_REQUESTED.store(false, Ordering::SeqCst);
    let client = http_client::ai_client();
    let mut last_usage: Option<ChatUsage> = None;

    // ── Agentic loop (same resilience pattern as the OpenAI path) ──────────
    const MAX_STREAM_RETRIES: usize = 2;

    for round in 0..=MAX_STREAM_RETRIES {
        if STOP_REQUESTED.load(Ordering::SeqCst) {
            break;
        }

        // Build the request body.
        let mut body = json!({
            "model": cfg.model,
            "max_tokens": 4096,
            "stream": true,
            "messages": messages,
        });
        if !system_parts.is_empty() {
            body["system"] = json!(system_parts.join("\n\n"));
        }
        if cfg.tools_enabled && !tools.is_empty() {
            body["tools"] = json!(tools);
        }

        // Send the request with Anthropic auth headers.
        let mut req = client.post(&url).json(&body);
        if !cfg.api_key.is_empty() {
            req = req.header("x-api-key", &cfg.api_key);
            req = req.header("anthropic-version", "2023-06-01");
        }

        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                let msg = format!("请求 AI 服务失败：{e}");
                emit_error(&app, msg.clone());
                return Err(msg);
            }
        };
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            let msg = format!("AI 服务返回错误 (HTTP {status})：{body}");
            emit_error(&app, msg.clone());
            return Err(msg);
        }

        // ── Parse the SSE stream ─────────────────────────────────────────────
        // Events are `event: <name>\ndata: <json>\n\n`. We only need the
        // `data:` payloads; event names are redundant with payload `type`.
        let mut stream = resp;
        let mut buf = String::new();
        let mut text_parts: Vec<String> = Vec::new();
        let mut tool_uses: Vec<AnthropicToolUse> = Vec::new();
        let mut current_tool_index: Option<usize> = None;
        let mut emitted_any_content = false;
        let mut stopped = false;
        let mut stream_clean = false;
        let mut round_usage: Option<ChatUsage> = None;
        let mut retryable = false;

        'stream: loop {
            if STOP_REQUESTED.load(Ordering::SeqCst) {
                stopped = true;
                break 'stream;
            }
            match stream.chunk().await {
                Ok(Some(chunk)) => {
                    buf.push_str(&String::from_utf8_lossy(&chunk));
                    while let Some(nl) = buf.find('\n') {
                        let line = buf[..nl].trim_end_matches('\r').to_string();
                        buf.drain(..=nl);
                        let Some(payload) = line.strip_prefix("data:") else {
                            continue;
                        };
                        let payload = payload.trim();
                        if payload.is_empty() {
                            continue;
                        }
                        let ev: AnthropicStreamEvent = match serde_json::from_str(payload) {
                            Ok(e) => e,
                            Err(_) => continue, // ignore non-JSON SSE lines
                        };
                        match ev.event_type.as_str() {
                            "message_start" => {
                                if let Some(u) = ev.message.and_then(|m| m.usage) {
                                    round_usage = Some(ChatUsage {
                                        prompt_tokens: u.input_tokens,
                                        completion_tokens: u.output_tokens,
                                        total_tokens: u.input_tokens + u.output_tokens,
                                        cached_tokens: 0,
                                    });
                                }
                            }
                            "content_block_start" => {
                                if let Some(block) = ev.content_block {
                                    if block.block_type == "tool_use" {
                                        tool_uses.push(AnthropicToolUse {
                                            id: block.id.clone().unwrap_or_default(),
                                            name: block.name.clone().unwrap_or_default(),
                                            arguments: String::new(),
                                        });
                                        current_tool_index = Some(tool_uses.len() - 1);
                                    }
                                }
                            }
                            "content_block_delta" => {
                                if let Some(d) = ev.delta {
                                    match d.delta_type.as_str() {
                                        "text_delta" => {
                                            if let Some(t) = d.text {
                                                text_parts.push(t.clone());
                                                emitted_any_content = true;
                                                let _ = app.emit("ai-chat-delta", t);
                                            }
                                        }
                                        "input_json_delta" => {
                                            if let Some(j) = d.partial_json {
                                                if let Some(idx) = current_tool_index {
                                                    if let Some(tu) = tool_uses.get_mut(idx) {
                                                        tu.arguments.push_str(&j);
                                                    }
                                                }
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            }
                            "content_block_stop" => {
                                current_tool_index = None;
                            }
                            "message_delta" => {
                                if let Some(u) = ev.usage {
                                    round_usage = Some(ChatUsage {
                                        prompt_tokens: u.input_tokens,
                                        completion_tokens: u.output_tokens,
                                        total_tokens: u.input_tokens + u.output_tokens,
                                        cached_tokens: 0,
                                    });
                                }
                            }
                            "message_stop" => {
                                stream_clean = true;
                                break 'stream;
                            }
                            "error" => {
                                if let Some(e) = ev.error {
                                    let msg = e
                                        .message
                                        .unwrap_or_else(|| "未知 Anthropic 错误".to_string());
                                    emit_error(&app, msg.clone());
                                    return Err(msg);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Ok(None) => break 'stream, // EOF without message_stop
                Err(e) => {
                    retryable = true;
                    warn!(target: "ai_chat", "Anthropic stream error: {e}");
                    break 'stream;
                }
            }
        }

        if let Some(u) = round_usage {
            last_usage = Some(u.clone());
            let _ = app.emit("ai-chat-usage", u);
        }

        // A clean message_stop means the model finished; stream errors mid-way
        // are retryable.
        if !stream_clean && !stopped {
            if retryable && round < MAX_STREAM_RETRIES {
                continue;
            }
            let msg = "Anthropic 流式响应中断".to_string();
            emit_error(&app, msg.clone());
            return Err(msg);
        }

        if stopped {
            break;
        }

        // ── Tool calls: execute and continue the loop ─────────────────────────
        if !tool_uses.is_empty() {
            // Append the assistant message with tool_use content blocks.
            let blocks: Vec<serde_json::Value> = tool_uses
                .iter()
                .map(|tu| {
                    json!({
                        "type": "tool_use",
                        "id": tu.id,
                        "name": tu.name,
                        "input": serde_json::from_str::<serde_json::Value>(&tu.arguments)
                            .unwrap_or_else(|_| json!({})),
                    })
                })
                .collect();
            messages.push(json!({
                "role": "assistant",
                "content": blocks,
            }));

            let tool_names: Vec<String> = tool_uses.iter().map(|t| t.name.clone()).collect();
            let _ = app.emit("ai-chat-tool", tool_names);

            // Execute each tool, append tool_result user message.
            let mut results: Vec<serde_json::Value> = Vec::new();
            for tu in &tool_uses {
                let args_for_ui = if tu.arguments.trim().is_empty() {
                    None
                } else {
                    Some(tu.arguments.clone())
                };
                let _ = app.emit(
                    "ai-chat-tool-call",
                    ToolCallEvent {
                        id: model_tool_call_event_id(&tu.id),
                        origin: ToolCallOrigin::Model,
                        name: tu.name.clone(),
                        arguments: args_for_ui.clone(),
                        status: ToolCallStatus::Running,
                        result: None,
                        error: None,
                        duration_ms: None,
                    },
                );
                let start = std::time::Instant::now();
                let result =
                    crate::services::ai_tools::execute_tool(&tool_ctx, &tu.name, &tu.arguments)
                        .await;
                let duration = start.elapsed().as_millis() as u64;
                // ToolResult carries `content` (JSON string) and `ok`. Errors
                // are returned as error-shaped JSON so the model can recover.
                let is_err = !result.ok;
                let _ = app.emit(
                    "ai-chat-tool-call",
                    ToolCallEvent {
                        id: model_tool_call_event_id(&tu.id),
                        origin: ToolCallOrigin::Model,
                        name: tu.name.clone(),
                        arguments: args_for_ui.clone(),
                        status: if is_err {
                            ToolCallStatus::Error
                        } else {
                            ToolCallStatus::Success
                        },
                        result: if is_err {
                            None
                        } else {
                            Some(truncate_for_display(&result.content))
                        },
                        error: if is_err {
                            Some(result.content.clone())
                        } else {
                            None
                        },
                        duration_ms: Some(duration),
                    },
                );
                results.push(json!({
                    "type": "tool_result",
                    "tool_use_id": tu.id,
                    "content": result.content,
                    "is_error": is_err,
                }));
            }
            messages.push(json!({
                "role": "user",
                "content": results,
            }));
            continue; // next round
        }

        // ── Final answer ──────────────────────────────────────────────────────
        if !emitted_any_content {
            let msg = "模型未返回任何内容，请检查模型名称或稍后重试".to_string();
            emit_error(&app, msg.clone());
            return Err(msg);
        }
        let full_text = text_parts.concat();
        let _ = app.emit(
            "ai-chat-done",
            serde_json::json!({ "content": full_text, "usage": last_usage }),
        );
        return Ok(());
    }

    // Loop exhausted without a final answer (e.g. all rounds were stopped).
    let msg = "对话未完成，请重试".to_string();
    emit_error(&app, msg.clone());
    Err(msg)
}
