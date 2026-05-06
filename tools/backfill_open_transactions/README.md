# backfill_open_transactions

一个独立的小工具，用于在 `transactions` 表中为已有的持仓补录**建仓（OPEN）记录**。

## 背景

在主程序中新建持仓时会自动创建一条 `OPEN` 类型的交易记录，记录初始建仓信息。
但对于**迁移数据**或**手工录入**的历史持仓，可能缺少这条记录。
本工具可扫描所有持仓并补全缺失的 OPEN 记录。

## 工作逻辑

对每一条非现金持仓（`holdings` 表中 `symbol` 不以 `$CASH-` 开头）：

| 情况 | 处理方式 |
|------|----------|
| 已有 `OPEN` 记录 | **跳过**（幂等，可重复运行） |
| 没有任何 BUY/SELL 交易记录 | 直接以持仓的 `shares` / `avg_cost` 创建 OPEN |
| 有 BUY/SELL 记录但没有 OPEN | 反推初始建仓数量和价格，再创建 OPEN |

### 反推公式

系统中 SELL 不改变 `avg_cost`，因此 `avg_cost_final` 等于所有买入（含初始 OPEN）的加权均价：

```
shares₀ = shares_final + Σsell_shares − Σbuy_shares

price₀  = ( (shares_final + Σsell_shares) × avg_cost_final
            − Σ(buy_shares × buy_price) ) / shares₀
```

反推得到的 OPEN 记录日期设为**最早一笔 BUY/SELL 交易的前一天**，保证时序正确。

## 构建与运行

> **前提**：已安装 [Rust 工具链](https://rustup.rs/)（1.70+）。
> 本工具不依赖 GTK / Tauri，可在任意平台独立编译。

```bash
cd tools/backfill_open_transactions

# 先预览（不写入数据库）
cargo run -- /path/to/portfolio.db --dry-run

# 确认无误后正式写入
cargo run -- /path/to/portfolio.db
```

### macOS 数据库默认路径

```
~/Library/Application Support/com.stock-portfolio-manager.app/portfolio.db
```

### Windows 数据库默认路径

```
%APPDATA%\com.stock-portfolio-manager.app\portfolio.db
```

## 输出示例

```
=== DRY-RUN 模式（不写入数据库）===

[预览] SH600036 (招商银行): OPEN — 1000 股 @ 35.5000 CNY | 成本 35500.00 | 日期 2023-12-31T10:00:00+00:00
[跳过] SH000001 (平安银行): 已存在 OPEN 建仓记录
[预览] HK.00700 (腾讯控股): OPEN — 100 股 @ 298.6543 HKD | 成本 29865.43 | 日期 2024-02-14T09:00:00+00:00
[跳过] US.AAPL (Apple): 现有买入交易已能解释全部持仓（反推初始持股数 ≈ 0.0000），无需创建建仓记录

=== 汇总 ===
将创建（预览）: 2
跳过:   2
错误:   0

以上为预览结果。去掉 --dry-run 参数后再次运行即可写入数据库。
```

## 运行测试

```bash
cargo test
```
