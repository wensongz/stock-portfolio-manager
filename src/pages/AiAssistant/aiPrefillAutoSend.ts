import type { AiToolContext } from "../../types";
import type { AiPrefillRequest } from "./prefill";

interface AiPrefillStagingDependencies {
  stageSkill: (skill: string) => void;
  stageTool: (tool: AiToolContext) => void;
}

export type AiSessionTransitionDecision =
  | "CLEAR"
  | "KEEP_IN_FLIGHT"
  | "LOAD"
  | "UNCHANGED";

export function decideAiSessionTransition(input: {
  nextSessionId: string | null;
  loadedSessionId: string | null;
  expectingSessionCreation: boolean;
}): AiSessionTransitionDecision {
  if (input.nextSessionId === null) return "CLEAR";
  if (input.loadedSessionId === input.nextSessionId) return "UNCHANGED";
  return input.expectingSessionCreation ? "KEEP_IN_FLIGHT" : "LOAD";
}

export function stageNonAutoAiPrefill(
  request: AiPrefillRequest | null,
  dependencies: AiPrefillStagingDependencies,
): string | null {
  if (!request || request.autoSend) return null;
  if (request.activeSkill) dependencies.stageSkill(request.activeSkill);
  if (request.toolContext) {
    dependencies.stageTool({
      name: request.toolContext.name,
      arguments: { ...request.toolContext.arguments },
    });
  }
  return request.prompt;
}

export function shouldAutoSendPrefill(input: {
  request: AiPrefillRequest | null;
  consumed: boolean;
  configured: boolean;
  sending: boolean;
}): boolean {
  return Boolean(
    input.request?.autoSend &&
    !input.consumed &&
    input.configured &&
    !input.sending,
  );
}

export interface AiPrefillAutoSendDependencies extends AiPrefillStagingDependencies {
  createSession: () => Promise<string>;
  sendMessage: (prompt: string, sessionId: string) => Promise<void>;
  touchSession: (sessionId: string) => Promise<void>;
  renameSession: (sessionId: string, prompt: string) => Promise<void>;
}

export async function runAiPrefillAutoSend(
  request: AiPrefillRequest,
  dependencies: AiPrefillAutoSendDependencies,
): Promise<string> {
  if (!request.autoSend || !request.activeSkill || !request.toolContext) {
    throw new Error("自动发送请求缺少可信技能或工具上下文");
  }
  dependencies.stageSkill(request.activeSkill);
  dependencies.stageTool({
    name: request.toolContext.name,
    arguments: { ...request.toolContext.arguments },
  });
  const sessionId = await dependencies.createSession();
  await dependencies.sendMessage(request.prompt, sessionId);
  await dependencies.touchSession(sessionId);
  await dependencies.renameSession(sessionId, request.prompt);
  return sessionId;
}
