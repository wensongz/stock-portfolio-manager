export type ThsTransactionType = "BUY" | "SELL" | "PAY";

export interface ThsCsvRow {
  key: string;
  raw?: string;
  external_id?: string | null;
  selected: boolean;
  transaction_type: ThsTransactionType;
  symbol: string;
  stock_name: string;
  traded_at: string;
  price: number;
  shares: number;
  total_amount: number;
  commission: number;
  notes?: string;
}

function splitCsvLine(line: string): string[] {
  const result: string[] = [];
  let current = "";
  let inQuotes = false;

  for (let i = 0; i < line.length; i++) {
    const ch = line[i];
    if (ch === '"') {
      if (inQuotes && line[i + 1] === '"') {
        current += '"';
        i++;
      } else {
        inQuotes = !inQuotes;
      }
    } else if (ch === "," && !inQuotes) {
      result.push(current);
      current = "";
    } else {
      current += ch;
    }
  }
  result.push(current);
  return result;
}

function parseNum(s: string | undefined): number {
  return parseFloat((s ?? "").replace(/,/g, "").trim());
}

function buildDateTime(date: string, time: string): string {
  const d = date.trim().replace(/[\/-]/g, "");
  const datePart =
    d.length === 8
      ? `${d.slice(0, 4)}-${d.slice(4, 6)}-${d.slice(6, 8)}`
      : date.trim();

  const t = time.trim().replace(/:/g, "");
  const timePart =
    t.length === 6
      ? `${t.slice(0, 2)}:${t.slice(2, 4)}:${t.slice(4, 6)}`
      : time.trim() || "09:30:00";

  return `${datePart}T${timePart}`;
}

function deriveSymbol(code: string, exchange: string): string {
  const c = code.trim();
  if (exchange.includes("上海") || exchange.toUpperCase().includes("SH")) {
    return `sh${c}`;
  }
  if (exchange.includes("深圳") || exchange.toUpperCase().includes("SZ")) {
    return `sz${c}`;
  }
  return c.startsWith("6") || c.startsWith("5") ? `sh${c}` : `sz${c}`;
}

function explicitTransactionType(operation: string): "BUY" | "SELL" | null {
  const normalized = operation.trim().toUpperCase();
  if (normalized === "买入" || normalized === "买" || normalized === "BUY") return "BUY";
  if (normalized === "卖出" || normalized === "卖" || normalized === "SELL") return "SELL";
  return null;
}

/** Parse A-share historical trades exported by THS and compatible brokers. */
export function parseThsCsv(text: string): ThsCsvRow[] {
  const stripped = text.startsWith("\uFEFF") ? text.slice(1) : text;
  const lines = stripped.split(/\r?\n/);

  let headerIdx = -1;
  for (let i = 0; i < lines.length; i++) {
    if (lines[i].includes("证券代码")) {
      headerIdx = i;
      break;
    }
  }
  if (headerIdx === -1) return [];

  const headers = splitCsvLine(lines[headerIdx]).map((h) => h.trim());
  const col = (name: string) => headers.indexOf(name);
  const firstCol = (...names: string[]) => {
    for (const name of names) {
      const index = col(name);
      if (index !== -1) return index;
    }
    return -1;
  };

  const iExternal = firstCol("成交编号", "成交序号", "交易编号");
  const iDate = firstCol("成交日期", "交易日期", "发生日期");
  const iTime = col("成交时间");
  const iCode = col("证券代码");
  const iName = col("证券名称");
  const iOp = firstCol("操作", "业务名称", "买卖标志");
  const iExchange = firstCol("交易所名称", "交易市场");
  const iPrice = firstCol("成交价格", "成交均价");
  const iShares = col("成交数量");
  const iAmount = col("成交金额");
  const iHappen = firstCol("发生金额", "清算金额");
  const iCommission = col("手续费");
  const iStamp = col("印花税");
  const iExtra = col("附加费");
  const iTransfer = col("过户费");

  if (iCode === -1 || iShares === -1) return [];

  const rows: ThsCsvRow[] = [];
  let idx = 0;

  for (let i = headerIdx + 1; i < lines.length; i++) {
    const line = lines[i];
    if (!line.trim()) continue;

    const cols = splitCsvLine(line);
    const get = (j: number) => (j !== -1 ? cols[j] ?? "" : "");
    const code = get(iCode).trim().replace(/^\d{1,5}$/, (s) => s.padStart(6, "0"));
    if (!/^\d{6}$/.test(code)) continue;

    const operation = iOp !== -1 ? get(iOp).trim() : "";
    if (operation === "上海存托服务费扣收") continue;
    const isDividend = operation === "红股派息"
      || operation === "股息入账"
      || operation === "红利";

    const shares = parseNum(get(iShares));
    if (!isDividend && (isNaN(shares) || shares === 0)) continue;

    const price = parseNum(get(iPrice));
    const tradeAmount = parseNum(get(iAmount));
    const happenAmt = parseNum(get(iHappen));

    let transaction_type: ThsTransactionType;
    let total_amount: number;

    if (isDividend) {
      const dividendAmount = !isNaN(happenAmt) && happenAmt !== 0
        ? happenAmt
        : tradeAmount;
      if (isNaN(dividendAmount) || dividendAmount === 0) continue;
      transaction_type = "PAY";
      total_amount = Math.abs(dividendAmount);
    } else {
      total_amount = isNaN(tradeAmount) || tradeAmount === 0
        ? Math.round(Math.abs(price) * Math.abs(shares) * 100) / 100
        : Math.abs(tradeAmount);
      transaction_type = explicitTransactionType(operation)
        ?? (!isNaN(happenAmt) && happenAmt > 0 ? "SELL" : "BUY");
    }

    const commission = Math.round(
      (
        (isNaN(parseNum(get(iCommission))) ? 0 : Math.abs(parseNum(get(iCommission)))) +
        (isNaN(parseNum(get(iStamp))) ? 0 : Math.abs(parseNum(get(iStamp)))) +
        (isNaN(parseNum(get(iExtra))) ? 0 : Math.abs(parseNum(get(iExtra)))) +
        (isNaN(parseNum(get(iTransfer))) ? 0 : Math.abs(parseNum(get(iTransfer))))
      ) * 100,
    ) / 100;

    const exchange = get(iExchange);
    const symbol = deriveSymbol(code, exchange);
    const stockName = get(iName).trim();
    const tradedAt = buildDateTime(get(iDate), get(iTime));

    rows.push({
      key: String(idx++),
      raw: line,
      external_id: /^0*$/.test(get(iExternal).trim()) ? null : get(iExternal).trim(),
      selected: true,
      transaction_type,
      symbol,
      stock_name: stockName || symbol,
      traded_at: tradedAt,
      price: isDividend ? 0 : Math.abs(price),
      shares: isDividend ? 0 : Math.abs(shares),
      total_amount,
      commission,
      notes: isDividend ? "分红派息" : undefined,
    });
  }

  return rows;
}
