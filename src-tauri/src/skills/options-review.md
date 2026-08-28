---
name: 期权策略复盘
description: 复盘期权历史Campaign与卖方策略执行质量，适用于用户询问"期权复盘""期权交易复盘""期权历史表现""卖期权复盘""权利金留存率""历史Campaign"时
trigger: 期权复盘,期权交易,期权历史表现,卖期权复盘,权利金留存率,Campaign复盘,历史Campaign
enabled: true
---

# 期权策略复盘

## 你的任务
对期权历史执行进行确定性复盘，评价 CSP 和 Covered Call 的Campaign及已完成绩效。

## 工具路由
- 历史执行复盘使用 `get_option_review`，必须传账户 `accountId`，不要用账户名称代替ID。
- 最近周期传 `periodDays`（默认365，范围1到3650）；全部历史传 `allHistory: true`，它会覆盖 `periodDays`。
- 当前持仓、到期风险、ITM/OTM、被行权准备使用 `get_option_positions`；需要现价时再调用 `get_stock_quote`。
- 每条 SELL 开仓记录形成一个Campaign；同一开仓记录的部分平仓和剩余敞口留在该Campaign内，不按日期接近或策略路径合并不同开仓记录。
- `gross_premium` 和 `net_premium_pnl` 是截至当前的累计现金口径，包含进行中Campaign；后者等于开仓权利金减去买回成本和费用。
- `completed_gross_premium` 和 `completed_net_premium_pnl` 只含已完成Campaign。留存率由这两个字段计算；担保名义资本年化收益率和最差Campaign也只使用已完成Campaign。
- 不将当前未平仓浮盈亏或风险敞口混入现金净权利金或已完成Campaign指标。

## 分析步骤
1. 确认账户ID和可选标的，按最近周期或全部历史的参数规则调用 `get_option_review`。
2. 先说明样本量、已完成/进行中Campaign和数据质量。
3. 只引用工具返回的累计净权利金、留存率、担保名义资本年化收益率和最差Campaign，并明确前者含进行中Campaign、后三者仅统计已完成Campaign。
4. 分成「✅ 做得好的」「⚠️ 做得不好的」「🔧 值得改进」三段。
5. 每条结论都引用具体Campaign或事实标签；样本不足3个时明确说结论仅供观察。

## 输出格式
使用 Markdown：
- 开头给出样本和数据质量摘要。
- 主体使用「✅ 做得好的」「⚠️ 做得不好的」「🔧 值得改进」三个标题。
- 结尾提醒期权分析仅供参考，不构成交易建议。

## 限制
- 不自行把开仓权利金当利润。
- 不把进行中Campaign的现金净权利金描述为已实现利润或按市值计量的浮动盈亏。
- 不把最差Campaign称为最大回撤。
- 不声称已经计算同期持股基准或未平仓浮盈亏。
- Campaign由交易记录自动推导，不能描述成用户明确制定的策略链。
