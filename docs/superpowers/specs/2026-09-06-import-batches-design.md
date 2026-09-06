# 导入批次、去重、对账与撤销

用户已确认实现审阅建议。范围为股票持仓/交易的券商 CSV、通用 CSV、OCR 导入；期权保留既有独立导入流程。

## 行为

- 后端统一批次服务，保留来源、文件名、原始内容、解析版本、原始行、标准化行、逐行状态和关联记录。
- 同一请求幂等；同一文件同一原始行和内容、同一来源成交编号自动识别明确重复；编号内容冲突拒绝。仅业务字段相同为疑似重复，默认不选择，可明确选择导入。不能把不同成交编号的同价成交丢掉。
- 先创建持久化预览，再选择行提交；一次提交使用外层事务，行级 savepoint 容许部分成功。再次提交只处理未成功行。提交时重新查重，避免并发预览穿透。
- 对账列出账户每个证券/现金的导入前后数量、变化、用户输入的券商余额和差额。未输入的项目显示未核对；只对输入项目判定。批次历史重启后仍可查看、重试和核对。
- 撤销采用保守冲突检查：当前账户持仓/交易必须与批次最后一次提交的后状态一致，否则拒绝。批次内新交易一并移除，持仓与现金恢复到第一次提交前状态，保留审计记录。已撤销批次不可重试。失败撤销必须零写入。失败重试也拒绝覆盖批次提交后的账户变更。
- 成功写入/撤销使受影响日期起的每日估值快照失效，保留季度笔记与快照供用户刷新。
- 不新增依赖，不读取或修改用户真实账户数据库；使用内存测试库。

## API（Tauri 外层参数 camelCase，模型 JSON snake_case）

`preview_import_batch({request}) -> ImportBatch`

Request: `{request_id, account_id, source, file_name, source_content, parser_version, kind: "transactions"|"holdings", rows: [{key, raw: JSON值, external_id?: string|null, data: object}]}`。
data 使用 snake_case：交易 `{symbol,name,market,currency,transaction_type,shares,price,total_amount,commission,traded_at,notes?}`；持仓 `{symbol,name,market,currency,shares,avg_cost,category_id?}`。account_id 只信任 request。

`apply_import_batch({batchId, rowKeys, allowSuspectedKeys}) -> ImportBatch`
`get_import_batch({batchId}) -> ImportBatch`
`list_import_batches({accountId: string|null}) -> ImportBatch[]`（最近 100 个，列表不含原始文件）
`undo_import_batch({batchId}) -> ImportBatch`
`reconcile_import_batch({batchId, balances: [{symbol, expected_shares}]}) -> ImportBatch`

ImportBatch: `{id,account_id,source,file_name,parser_version,kind,status: "preview"|"applied"|"undone",created_at,rows: [{key,raw,external_id,data,status: "ready"|"suspected"|"duplicate"|"failed"|"imported",error: string|null,record_id: string|null}],reconciliation: [{symbol,currency,before_shares,after_shares,expected_shares: number|null,difference: number|null}],can_undo: boolean,conflict: string|null}`。

通用 CSV 增加 `preview_csv_import_batch({content,dataType,accountId,fileName,requestId}) -> ImportBatch`，保留现有解析错误提示。返回所有有效行，不截断预览提交数据。旧 confirm_import 保留兼容但转发批次服务。

## 架构选择

选择后端批次服务而非前端去重（多个入口和重启会绕过），选择受保护的前后状态恢复而非全库恢复（避免覆盖其他账户）。后续可放宽撤销依赖范围，但本版本明确说明限制。

## 验收

重复请求/文件/成交编号不重复记账；疑似重复可保留合法多笔成交；失败行重试不重复成功行；撤销恢复现金、股数和成本；后续变更阻止撤销；撤销中失败原子回滚；缺少余额不误报对账成功；数据库升级保留旧记录。
