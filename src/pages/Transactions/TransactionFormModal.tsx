import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AutoComplete, Col, DatePicker, Form, Input, InputNumber, Modal, Row, Select, Typography, message } from "antd";
import dayjs from "dayjs";
import { invoke } from "@tauri-apps/api/core";
import { useAccountStore } from "../../stores/accountStore";
import { useTransactionStore } from "../../stores/transactionStore";
import { useQuoteStore } from "../../stores/quoteStore";
import type { Currency, Holding, Market, QuoteCommandResult, StockQuote, Transaction, TransactionType } from "../../types";

interface Props {
  open: boolean;
  transaction?: Transaction | null;
  initialAccountId?: string;
  onClose: () => void;
  onSaved?: (transaction: Transaction) => void | Promise<void>;
}

interface TransactionFormValues {
  accountId: string;
  symbol: string;
  name: string;
  market: Market;
  transactionType: TransactionType | "CASH_IN" | "CASH_OUT";
  shares: number;
  price: number;
  totalAmount: number;
  commission: number;
  currency: Currency;
  tradedAt: dayjs.Dayjs;
  notes?: string;
}

const CASH_SYMBOL_PREFIX = "$CASH-";

const marketCurrencyMap: Record<Market, Currency> = {
  US: "USD",
  CN: "CNY",
  HK: "HKD",
};

function shareInputProps(market?: Market) {
  return market === "US"
    ? { min: 0.000001, precision: 6, placeholder: "交易股数" }
    : { min: 1, precision: 0, placeholder: "交易股数" };
}

// Default traded time: 1 hour after market open
// US: 9:30 ET → 10:30 ET (use local hour 10, min 30)
// CN: 9:30 CST → 10:30 CST (use local hour 10, min 30)
// HK: 9:30 HKT → 10:30 HKT (use local hour 10, min 30)
const marketDefaultTime: Record<Market, { hour: number; minute: number }> = {
  US: { hour: 10, minute: 30 },
  CN: { hour: 10, minute: 30 },
  HK: { hour: 10, minute: 30 },
};

function getDefaultTradedAt(market?: Market): dayjs.Dayjs {
  const time = market ? marketDefaultTime[market] : { hour: 10, minute: 30 };
  return dayjs().hour(time.hour).minute(time.minute).second(0);
}

export default function TransactionFormModal({
  open,
  transaction = null,
  initialAccountId,
  onClose,
  onSaved,
}: Props) {
  const { accounts, fetchAccounts } = useAccountStore();
  const { createTransaction, updateTransaction } = useTransactionStore();
  const [form] = Form.useForm<TransactionFormValues>();
  const watchedType = Form.useWatch("transactionType", form);
  const selectedFormMarket = Form.useWatch("market", form) as Market | undefined;
  const isDividend = watchedType === "PAY";
  const isStockTransfer = watchedType === "STOCK_IN" || watchedType === "STOCK_OUT";
  const isStockOut = watchedType === "STOCK_OUT";
  const isCashTxn = watchedType === "CASH_IN" || watchedType === "CASH_OUT";
  const [accountHoldings, setAccountHoldings] = useState<Holding[]>([]);
  const [symbolSearching, setSymbolSearching] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const session = useRef(0);
  const holdingsRequest = useRef(0);
  const symbolRequest = useRef(0);
  const submittingRef = useRef(false);

  const loadAccountHoldings = useCallback(async (accountId: string) => {
    const generation = session.current;
    const request = ++holdingsRequest.current;
    setAccountHoldings([]);
    try {
      const holdings = await invoke<Holding[]>("get_holdings", { accountId });
      if (session.current === generation && holdingsRequest.current === request) {
        setAccountHoldings(holdings);
      }
    } catch {
      if (session.current === generation && holdingsRequest.current === request) {
        setAccountHoldings([]);
      }
    }
  }, []);

  useEffect(() => {
    const generation = ++session.current;
    holdingsRequest.current += 1;
    symbolRequest.current += 1;
    submittingRef.current = false;
    setSubmitting(false);
    setSymbolSearching(false);
    setAccountHoldings([]);
    if (!open) return;

    form.resetFields();
    if (transaction) {
      const isCashRecord = transaction.symbol.startsWith(CASH_SYMBOL_PREFIX);
      form.setFieldsValue({
        accountId: transaction.account_id,
        symbol: transaction.symbol,
        name: transaction.name,
        market: transaction.market,
        transactionType: isCashRecord && transaction.transaction_type !== "OPEN"
          ? transaction.transaction_type === "BUY" ? "CASH_IN" : "CASH_OUT"
          : transaction.transaction_type,
        shares: transaction.shares,
        price: transaction.price,
        totalAmount: transaction.total_amount,
        commission: transaction.commission,
        currency: transaction.currency,
        tradedAt: dayjs(transaction.traded_at),
        notes: transaction.notes ?? undefined,
      });
      void loadAccountHoldings(transaction.account_id);
    } else {
      form.setFieldsValue({ accountId: initialAccountId, tradedAt: getDefaultTradedAt(), commission: 0 });
      if (initialAccountId) {
        const account = useAccountStore.getState().accounts.find((item) => item.id === initialAccountId);
        if (account) {
          form.setFieldsValue({ market: account.market, currency: marketCurrencyMap[account.market], tradedAt: getDefaultTradedAt(account.market) });
        }
        void loadAccountHoldings(initialAccountId);
      }
    }
    if (useAccountStore.getState().accounts.length === 0) {
      void fetchAccounts().then(() => {
        if (session.current !== generation || transaction || !initialAccountId || form.getFieldValue("accountId") !== initialAccountId) return;
        const account = useAccountStore.getState().accounts.find((item) => item.id === initialAccountId);
        if (account && !form.getFieldValue("market")) {
          form.setFieldsValue({ market: account.market, currency: marketCurrencyMap[account.market] });
        }
      });
    }
    return () => {
      session.current += 1;
      holdingsRequest.current += 1;
      symbolRequest.current += 1;
    };
  }, [open, transaction, initialAccountId, form, fetchAccounts, loadAccountHoldings]);

  const handleAccountChange = useCallback((accountId: string) => {
    symbolRequest.current += 1;
    setSymbolSearching(false);
    const account = accounts.find((item) => item.id === accountId);
    if (account) {
      form.setFieldsValue({
        market: account.market,
        currency: marketCurrencyMap[account.market],
        tradedAt: getDefaultTradedAt(account.market),
      });
    }
    void loadAccountHoldings(accountId);
  }, [accounts, form, loadAccountHoldings]);

  const symbolOptions = useMemo(() => accountHoldings
    .filter((holding) => holding.shares > 0)
    .map((holding) => ({ value: holding.symbol, label: `${holding.symbol} - ${holding.name} (持仓: ${holding.shares})` })), [accountHoldings]);

  const handleSymbolSelect = useCallback((value: string) => {
    const holding = accountHoldings.find((item) => item.symbol === value);
    if (holding) form.setFieldsValue({ name: holding.name, market: holding.market, currency: holding.currency });
  }, [accountHoldings, form]);

  const handleSymbolBlur = useCallback(async () => {
    const symbol = form.getFieldValue("symbol");
    const name = form.getFieldValue("name");
    const market = form.getFieldValue("market");
    const accountId = form.getFieldValue("accountId");
    if (!symbol || name || !market) return;
    const holding = accountHoldings.find((item) => item.symbol.toUpperCase() === symbol.toUpperCase());
    if (holding) {
      form.setFieldsValue({ name: holding.name });
      return;
    }
    const generation = session.current;
    const request = ++symbolRequest.current;
    setSymbolSearching(true);
    try {
      const outcome = await invoke<QuoteCommandResult<StockQuote[]>>("get_real_time_quotes", {
        symbols: [[symbol, market]], forceRefresh: false,
      });
      useQuoteStore.getState().applyQuoteMetadata(outcome);
      if (session.current === generation && symbolRequest.current === request &&
          form.getFieldValue("symbol") === symbol && form.getFieldValue("market") === market &&
          form.getFieldValue("accountId") === accountId && !form.getFieldValue("name") && outcome.data[0]?.name) {
        form.setFieldsValue({ name: outcome.data[0].name });
      }
    } catch {
      // The stock name can also be entered manually.
    } finally {
      if (session.current === generation && symbolRequest.current === request) setSymbolSearching(false);
    }
  }, [accountHoldings, form]);

  const handleAmountFieldChange = useCallback(() => {
    const shares = form.getFieldValue("shares");
    const price = form.getFieldValue("price");
    if (typeof shares === "number" && typeof price === "number" && shares > 0 && price > 0) {
      form.setFieldsValue({ totalAmount: Math.round(shares * price * 100) / 100 });
    }
  }, [form]);

  const handleSubmit = async (values: TransactionFormValues) => {
    if (submittingRef.current) return;
    const isCash = values.transactionType === "CASH_IN" || values.transactionType === "CASH_OUT";
    if (isCash && !values.currency) {
      message.error("请先选择币种");
      return;
    }
    const submittedValues = values.transactionType === "PAY" ? { ...values, shares: 0, price: 0 } : values;
    const stockSubmitted = values.transactionType === "STOCK_IN"
      ? { ...submittedValues, totalAmount: values.shares * values.price, commission: 0 }
      : values.transactionType === "STOCK_OUT"
        ? { ...submittedValues, price: 0, totalAmount: 0, commission: 0 }
        : submittedValues;
    const payload = {
      ...stockSubmitted,
      ...(isCash ? {
        transactionType: values.transactionType === "CASH_IN" ? "BUY" as const : "SELL" as const,
        symbol: `${CASH_SYMBOL_PREFIX}${values.currency}`,
        name: `现金 (${values.currency})`,
        shares: 0,
        price: 0,
        commission: 0,
      } : { transactionType: stockSubmitted.transactionType as TransactionType }),
      tradedAt: values.tradedAt.toISOString(),
    };
    const generation = session.current;
    submittingRef.current = true;
    setSubmitting(true);
    let saved: Transaction;
    try {
      saved = transaction
        ? await updateTransaction({ id: transaction.id, ...payload })
        : await createTransaction(payload);
    } catch (err) {
      message.error(`操作失败: ${err}`);
      if (session.current === generation) {
        submittingRef.current = false;
        setSubmitting(false);
      }
      return;
    }
    if (session.current !== generation) return;
    try {
      // Keep the editor in its saving state until the detail view has fresh data.
      await onSaved?.(saved);
      message.success(transaction ? "交易记录更新成功" : "交易记录添加成功");
    } catch (err) {
      // The write already succeeded; do not invite another submission.
      message.warning(`交易记录已保存，但刷新明细失败: ${err}`);
    }
    if (session.current === generation) onClose();
  };

  return (
    <Modal
      title={transaction ? "编辑交易记录" : "录入交易记录"}
      open={open}
      onOk={() => form.submit()}
      onCancel={() => { if (!submittingRef.current) onClose(); }}
      confirmLoading={submitting}
      cancelButtonProps={{ disabled: submitting }}
      closable={!submitting}
      maskClosable={!submitting}
      keyboard={!submitting}
      okText="确认"
      cancelText="取消"
      width={640}
    >
      <Form form={form} layout="vertical" onFinish={handleSubmit}>
        <Form.Item name="accountId" label="证券账户" style={{ marginBottom: 12 }}
          rules={[{ required: true, message: "请选择账户" }]}>
          <Select placeholder="选择证券账户" onChange={handleAccountChange}>
            {accounts.map((a) => (
              <Select.Option key={a.id} value={a.id}>
                [{a.market}] {a.name}
              </Select.Option>
            ))}
          </Select>
        </Form.Item>
        {!isCashTxn && (
          <Row gutter={12}>
            <Col span={12}>
              <Form.Item name="symbol" label="股票代码" style={{ marginBottom: 12 }}
                rules={[{ required: true, message: "请输入股票代码" }]}>
                <AutoComplete
                  options={symbolOptions}
                  placeholder="输入或选择股票代码"
                  onSelect={handleSymbolSelect}
                  onBlur={handleSymbolBlur}
                  filterOption={(inputValue, option) =>
                    (option?.value?.toString().toUpperCase().indexOf(inputValue.toUpperCase()) ?? -1) >= 0 ||
                    (option?.label?.toString().toUpperCase().indexOf(inputValue.toUpperCase()) ?? -1) >= 0
                  }
                />
              </Form.Item>
            </Col>
            <Col span={12}>
              <Form.Item name="name" label="股票名称" style={{ marginBottom: 12 }}
                rules={[{ required: true, message: "请输入股票名称" }]}>
                <Input placeholder="如：苹果" disabled={symbolSearching} />
              </Form.Item>
            </Col>
          </Row>
        )}
        <Row gutter={12}>
          <Col span={12}>
            <Form.Item name="transactionType" label="交易类型" style={{ marginBottom: 12 }}
              rules={[{ required: true, message: "请选择交易类型" }]}>
              <Select placeholder="选择交易类型">
                <Select.Option value="BUY">买入</Select.Option>
                <Select.Option value="SELL">卖出</Select.Option>
                <Select.Option value="PAY">分红</Select.Option>
                <Select.Option value="CASH_IN">存入现金</Select.Option>
                <Select.Option value="CASH_OUT">提取现金</Select.Option>
                <Select.Option value="STOCK_IN">存入股票</Select.Option>
                <Select.Option value="STOCK_OUT">提取股票</Select.Option>
              </Select>
            </Form.Item>
          </Col>
          <Col span={12}>
            <Form.Item name="tradedAt" label={isStockTransfer ? "发生时间" : "成交时间"} style={{ marginBottom: 12 }}
              rules={[{ required: true, message: "请选择成交时间" }]}>
              <DatePicker showTime style={{ width: "100%" }} />
            </Form.Item>
          </Col>
        </Row>
        {isStockTransfer && (
          <Typography.Paragraph type="secondary">
            {isStockOut ? "按原持仓成本提取股票，不增加现金、不计入卖出收益。提取数量不能超过发生时间的持仓。" : "按填写的每股成本存入股票并计算持仓均价，不扣除现金。"}
          </Typography.Paragraph>
        )}
        {!isDividend && !isCashTxn && (
          <Row gutter={12}>
            <Col span={12}>
              <Form.Item name="shares" label={isStockTransfer ? (isStockOut ? "提取股数" : "存入股数") : "交易股数"} style={{ marginBottom: 12 }}
                rules={[{ required: true, message: "请输入交易股数" }]}>
                <InputNumber
                  {...shareInputProps(selectedFormMarket)}
                  style={{ width: "100%" }}
                  onChange={handleAmountFieldChange} />
              </Form.Item>
            </Col>
            {!isStockOut && <Col span={12}>
              <Form.Item name="price" label={watchedType === "STOCK_IN" ? "每股成本" : "成交价格"} style={{ marginBottom: 12 }}
                rules={[{ required: true, message: watchedType === "STOCK_IN" ? "请输入每股成本" : "请输入成交价格" }]}>
                <InputNumber min={0} precision={4} style={{ width: "100%" }}
                  onChange={handleAmountFieldChange} />
              </Form.Item>
            </Col>}
          </Row>
        )}
        {!isStockTransfer && <Row gutter={12}>
          <Col span={12}>
            <Form.Item name="totalAmount" label="成交总额" style={{ marginBottom: 12 }}
              rules={[{ required: true, message: "请输入成交总额" }]}>
              <InputNumber min={0} precision={2} style={{ width: "100%" }} />
            </Form.Item>
          </Col>
          <Col span={12}>
            <Form.Item name="commission" label="手续费" style={{ marginBottom: 12 }}>
              <InputNumber min={0} precision={2} style={{ width: "100%" }} />
            </Form.Item>
          </Col>
        </Row>}
        <Row gutter={12}>
          <Col span={12}>
            <Form.Item name="market" label="市场" style={{ marginBottom: 12 }}
              rules={[{ required: true, message: "请选择市场" }]}>
              <Select placeholder="选择市场">
                <Select.Option value="US">🇺🇸 美股</Select.Option>
                <Select.Option value="CN">🇨🇳 A股</Select.Option>
                <Select.Option value="HK">🇭🇰 港股</Select.Option>
              </Select>
            </Form.Item>
          </Col>
          <Col span={12}>
            <Form.Item name="currency" label="币种" style={{ marginBottom: 12 }}
              rules={[{ required: true, message: "请选择币种" }]}>
              <Select placeholder="选择币种">
                <Select.Option value="USD">USD 美元</Select.Option>
                <Select.Option value="CNY">CNY 人民币</Select.Option>
                <Select.Option value="HKD">HKD 港元</Select.Option>
              </Select>
            </Form.Item>
          </Col>
        </Row>
        <Form.Item name="notes" label="备注（可选）" style={{ marginBottom: 0 }}>
          <Input.TextArea rows={2} placeholder="交易备注" />
        </Form.Item>
      </Form>
    </Modal>
  );
}
