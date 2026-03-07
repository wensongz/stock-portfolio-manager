import { useEffect, useState } from "react";
import {
  Typography,
  Button,
  Table,
  Space,
  Modal,
  Form,
  Input,
  Select,
  InputNumber,
  Tag,
  Popconfirm,
  message,
  DatePicker,
} from "antd";
import { PlusOutlined } from "@ant-design/icons";
import dayjs from "dayjs";
import { useTransactionStore } from "../../stores/transactionStore";
import { useAccountStore } from "../../stores/accountStore";
import type { Transaction, Market, Currency, TransactionType } from "../../types";

const { Title } = Typography;

const marketColors: Record<Market, string> = {
  US: "blue",
  CN: "red",
  HK: "green",
};

export default function TransactionsPage() {
  const { transactions, loading, fetchTransactions, createTransaction, deleteTransaction } =
    useTransactionStore();
  const { accounts, fetchAccounts } = useAccountStore();
  const [modalOpen, setModalOpen] = useState(false);
  const [form] = Form.useForm();

  useEffect(() => {
    fetchTransactions();
    fetchAccounts();
  }, [fetchTransactions, fetchAccounts]);

  const handleSubmit = async (values: {
    account_id: string;
    symbol: string;
    name: string;
    market: Market;
    transaction_type: TransactionType;
    shares: number;
    price: number;
    total_amount: number;
    commission: number;
    currency: Currency;
    traded_at: dayjs.Dayjs;
    notes?: string;
  }) => {
    try {
      await createTransaction({
        ...values,
        traded_at: values.traded_at.toISOString(),
      });
      message.success("交易记录添加成功");
      setModalOpen(false);
      form.resetFields();
    } catch (err) {
      message.error(`操作失败: ${err}`);
    }
  };

  const handleDelete = async (id: string) => {
    try {
      await deleteTransaction(id);
      message.success("交易记录删除成功");
    } catch (err) {
      message.error(`删除失败: ${err}`);
    }
  };

  const accountMap = Object.fromEntries(accounts.map((a) => [a.id, a.name]));

  const columns = [
    {
      title: "日期",
      dataIndex: "traded_at",
      key: "traded_at",
      render: (date: string) => dayjs(date).format("YYYY-MM-DD HH:mm"),
    },
    {
      title: "股票",
      key: "stock",
      render: (_: unknown, record: Transaction) => (
        <Space>
          <Tag color={marketColors[record.market]}>{record.market}</Tag>
          <strong>{record.symbol}</strong>
          <span className="text-gray-500 text-sm">{record.name}</span>
        </Space>
      ),
    },
    {
      title: "账户",
      dataIndex: "account_id",
      key: "account_id",
      render: (id: string) => accountMap[id] || id,
    },
    {
      title: "类型",
      dataIndex: "transaction_type",
      key: "transaction_type",
      render: (type: TransactionType) => (
        <Tag color={type === "BUY" ? "green" : "red"}>
          {type === "BUY" ? "买入" : "卖出"}
        </Tag>
      ),
    },
    {
      title: "股数",
      dataIndex: "shares",
      key: "shares",
      render: (v: number) => v.toLocaleString(),
    },
    {
      title: "价格",
      dataIndex: "price",
      key: "price",
      render: (v: number, record: Transaction) => `${record.currency} ${v.toFixed(2)}`,
    },
    {
      title: "总金额",
      dataIndex: "total_amount",
      key: "total_amount",
      render: (v: number, record: Transaction) => `${record.currency} ${v.toFixed(2)}`,
    },
    {
      title: "操作",
      key: "action",
      render: (_: unknown, record: Transaction) => (
        <Popconfirm
          title="确认删除该交易记录？"
          onConfirm={() => handleDelete(record.id)}
          okText="确认"
          cancelText="取消"
        >
          <Button type="link" size="small" danger>
            删除
          </Button>
        </Popconfirm>
      ),
    },
  ];

  return (
    <div>
      <div className="flex justify-between items-center mb-4">
        <Title level={2} className="!mb-0">
          交易记录
        </Title>
        <Button
          type="primary"
          icon={<PlusOutlined />}
          onClick={() => {
            form.resetFields();
            setModalOpen(true);
          }}
        >
          录入交易
        </Button>
      </div>

      <Table
        dataSource={transactions}
        columns={columns}
        rowKey="id"
        loading={loading}
        pagination={{ pageSize: 20 }}
      />

      <Modal
        title="录入交易记录"
        open={modalOpen}
        onOk={() => form.submit()}
        onCancel={() => {
          setModalOpen(false);
          form.resetFields();
        }}
        okText="确认"
        cancelText="取消"
        width={640}
      >
        <Form form={form} layout="vertical" onFinish={handleSubmit}
          initialValues={{ traded_at: dayjs(), commission: 0 }}>
          <Form.Item name="account_id" label="证券账户"
            rules={[{ required: true, message: "请选择账户" }]}>
            <Select placeholder="选择证券账户">
              {accounts.map((a) => (
                <Select.Option key={a.id} value={a.id}>
                  [{a.market}] {a.name}
                </Select.Option>
              ))}
            </Select>
          </Form.Item>
          <Form.Item name="symbol" label="股票代码"
            rules={[{ required: true, message: "请输入股票代码" }]}>
            <Input placeholder="如：AAPL" />
          </Form.Item>
          <Form.Item name="name" label="股票名称"
            rules={[{ required: true, message: "请输入股票名称" }]}>
            <Input placeholder="如：苹果" />
          </Form.Item>
          <Form.Item name="market" label="市场"
            rules={[{ required: true, message: "请选择市场" }]}>
            <Select placeholder="选择市场">
              <Select.Option value="US">🇺🇸 美股</Select.Option>
              <Select.Option value="CN">🇨🇳 A股</Select.Option>
              <Select.Option value="HK">🇭🇰 港股</Select.Option>
            </Select>
          </Form.Item>
          <Form.Item name="transaction_type" label="交易类型"
            rules={[{ required: true, message: "请选择交易类型" }]}>
            <Select placeholder="买入 / 卖出">
              <Select.Option value="BUY">买入</Select.Option>
              <Select.Option value="SELL">卖出</Select.Option>
            </Select>
          </Form.Item>
          <Form.Item name="shares" label="交易股数"
            rules={[{ required: true, message: "请输入交易股数" }]}>
            <InputNumber min={0} precision={2} style={{ width: "100%" }} />
          </Form.Item>
          <Form.Item name="price" label="成交价格"
            rules={[{ required: true, message: "请输入成交价格" }]}>
            <InputNumber min={0} precision={4} style={{ width: "100%" }} />
          </Form.Item>
          <Form.Item name="total_amount" label="成交总额"
            rules={[{ required: true, message: "请输入成交总额" }]}>
            <InputNumber min={0} precision={2} style={{ width: "100%" }} />
          </Form.Item>
          <Form.Item name="commission" label="手续费">
            <InputNumber min={0} precision={2} style={{ width: "100%" }} />
          </Form.Item>
          <Form.Item name="currency" label="币种"
            rules={[{ required: true, message: "请选择币种" }]}>
            <Select placeholder="选择币种">
              <Select.Option value="USD">USD 美元</Select.Option>
              <Select.Option value="CNY">CNY 人民币</Select.Option>
              <Select.Option value="HKD">HKD 港元</Select.Option>
            </Select>
          </Form.Item>
          <Form.Item name="traded_at" label="成交时间"
            rules={[{ required: true, message: "请选择成交时间" }]}>
            <DatePicker showTime style={{ width: "100%" }} />
          </Form.Item>
          <Form.Item name="notes" label="备注（可选）">
            <Input.TextArea rows={3} placeholder="交易备注" />
          </Form.Item>
        </Form>
      </Modal>
    </div>
  );
}
