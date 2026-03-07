import { useState, useEffect, useCallback } from 'react';
import {
  Table,
  Button,
  Modal,
  Form,
  Input,
  Select,
  Space,
  Tag,
  message,
  Popconfirm,
  Card,
  InputNumber,
  DatePicker,
} from 'antd';
import { PlusOutlined, DeleteOutlined } from '@ant-design/icons';
import dayjs from 'dayjs';
import { transactionApi, accountApi } from '../api';
import type { Transaction, Account } from '../types';
import { MARKET_LABELS, CURRENCY_MAP } from '../types';

export default function TransactionsPage() {
  const [transactions, setTransactions] = useState<Transaction[]>([]);
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [loading, setLoading] = useState(false);
  const [modalOpen, setModalOpen] = useState(false);
  const [filterAccountId, setFilterAccountId] = useState<string | undefined>();
  const [filterSymbol, setFilterSymbol] = useState<string | undefined>();
  const [form] = Form.useForm();

  const loadData = useCallback(async () => {
    setLoading(true);
    try {
      const [t, a] = await Promise.all([
        transactionApi.list(filterAccountId, filterSymbol),
        accountApi.list(),
      ]);
      setTransactions(t);
      setAccounts(a);
    } catch (e) {
      message.error(`Failed to load data: ${e}`);
    } finally {
      setLoading(false);
    }
  }, [filterAccountId, filterSymbol]);

  useEffect(() => {
    loadData();
  }, [loadData]);

  const handleCreate = () => {
    form.resetFields();
    form.setFieldsValue({ type: 'BUY', traded_at: dayjs() });
    setModalOpen(true);
  };

  const handleDelete = async (id: string) => {
    try {
      await transactionApi.delete(id);
      message.success('Transaction deleted');
      loadData();
    } catch (e) {
      message.error(`Failed to delete: ${e}`);
    }
  };

  const handleSubmit = async () => {
    try {
      const values = await form.validateFields();
      const account = accounts.find((a) => a.id === values.account_id);
      if (!account) return;

      await transactionApi.create({
        account_id: values.account_id,
        symbol: values.symbol,
        name: values.name,
        market: account.market,
        type: values.type,
        shares: values.shares,
        price: values.price,
        commission: values.commission,
        currency: CURRENCY_MAP[account.market] as 'USD' | 'CNY' | 'HKD',
        traded_at: values.traded_at.format('YYYY-MM-DD HH:mm:ss'),
        notes: values.notes,
      });
      message.success('Transaction created — holding updated automatically');
      setModalOpen(false);
      loadData();
    } catch (e) {
      if (typeof e === 'string') message.error(e);
    }
  };

  const getAccountName = (accountId: string) => {
    const acc = accounts.find((a) => a.id === accountId);
    return acc ? acc.name : accountId;
  };

  const columns = [
    {
      title: 'Date',
      dataIndex: 'traded_at',
      key: 'traded_at',
      render: (v: string) => dayjs(v).format('YYYY-MM-DD'),
    },
    {
      title: 'Type',
      dataIndex: 'type',
      key: 'type',
      render: (type: string) => (
        <Tag color={type === 'BUY' ? 'green' : 'red'}>{type}</Tag>
      ),
    },
    {
      title: 'Symbol',
      dataIndex: 'symbol',
      key: 'symbol',
      render: (v: string) => <strong>{v}</strong>,
    },
    {
      title: 'Name',
      dataIndex: 'name',
      key: 'name',
    },
    {
      title: 'Account',
      dataIndex: 'account_id',
      key: 'account_id',
      render: (id: string) => getAccountName(id),
    },
    {
      title: 'Market',
      dataIndex: 'market',
      key: 'market',
      render: (market: string) => <Tag>{MARKET_LABELS[market]}</Tag>,
    },
    {
      title: 'Shares',
      dataIndex: 'shares',
      key: 'shares',
      align: 'right' as const,
      render: (v: number) => v.toLocaleString(),
    },
    {
      title: 'Price',
      dataIndex: 'price',
      key: 'price',
      align: 'right' as const,
      render: (v: number) => v.toFixed(2),
    },
    {
      title: 'Total',
      dataIndex: 'total_amount',
      key: 'total_amount',
      align: 'right' as const,
      render: (v: number) => v.toFixed(2),
    },
    {
      title: 'Commission',
      dataIndex: 'commission',
      key: 'commission',
      align: 'right' as const,
      render: (v: number) => v.toFixed(2),
    },
    {
      title: 'Notes',
      dataIndex: 'notes',
      key: 'notes',
      ellipsis: true,
    },
    {
      title: 'Actions',
      key: 'actions',
      render: (_: unknown, record: Transaction) => (
        <Popconfirm
          title="Delete this transaction?"
          description="Note: This will NOT reverse the holding changes."
          onConfirm={() => handleDelete(record.id)}
        >
          <Button type="link" danger icon={<DeleteOutlined />}>
            Delete
          </Button>
        </Popconfirm>
      ),
    },
  ];

  return (
    <Card
      title="Transactions"
      extra={
        <Space>
          <Select
            allowClear
            placeholder="Filter by account"
            style={{ width: 200 }}
            value={filterAccountId}
            onChange={(v) => setFilterAccountId(v)}
            options={accounts.map((a) => ({
              label: `${a.name} (${MARKET_LABELS[a.market]})`,
              value: a.id,
            }))}
          />
          <Input
            allowClear
            placeholder="Filter by symbol"
            style={{ width: 140 }}
            value={filterSymbol}
            onChange={(e) => setFilterSymbol(e.target.value || undefined)}
          />
          <Button type="primary" icon={<PlusOutlined />} onClick={handleCreate}>
            New Transaction
          </Button>
        </Space>
      }
    >
      <Table
        columns={columns}
        dataSource={transactions}
        rowKey="id"
        loading={loading}
        pagination={{ pageSize: 20 }}
        scroll={{ x: 1200 }}
      />

      <Modal
        title="New Transaction"
        open={modalOpen}
        onOk={handleSubmit}
        onCancel={() => setModalOpen(false)}
        width={520}
      >
        <Form form={form} layout="vertical">
          <Form.Item
            name="account_id"
            label="Account"
            rules={[{ required: true, message: 'Please select an account' }]}
          >
            <Select
              options={accounts.map((a) => ({
                label: `${a.name} (${MARKET_LABELS[a.market]})`,
                value: a.id,
              }))}
            />
          </Form.Item>

          <Form.Item
            name="type"
            label="Transaction Type"
            rules={[{ required: true }]}
          >
            <Select
              options={[
                { label: '🟢 Buy', value: 'BUY' },
                { label: '🔴 Sell', value: 'SELL' },
              ]}
            />
          </Form.Item>

          <Form.Item
            name="symbol"
            label="Stock Symbol"
            rules={[{ required: true, message: 'Please enter symbol' }]}
          >
            <Input placeholder="e.g., AAPL, 600519.SH" />
          </Form.Item>

          <Form.Item
            name="name"
            label="Stock Name"
            rules={[{ required: true, message: 'Please enter stock name' }]}
          >
            <Input placeholder="e.g., Apple Inc., 贵州茅台" />
          </Form.Item>

          <Space style={{ width: '100%' }} size="middle">
            <Form.Item
              name="shares"
              label="Shares"
              rules={[{ required: true, message: 'Required' }]}
              style={{ flex: 1 }}
            >
              <InputNumber min={0.0001} style={{ width: '100%' }} />
            </Form.Item>

            <Form.Item
              name="price"
              label="Price"
              rules={[{ required: true, message: 'Required' }]}
              style={{ flex: 1 }}
            >
              <InputNumber min={0} step={0.01} style={{ width: '100%' }} />
            </Form.Item>
          </Space>

          <Form.Item name="commission" label="Commission">
            <InputNumber min={0} step={0.01} style={{ width: '100%' }} />
          </Form.Item>

          <Form.Item
            name="traded_at"
            label="Trade Date"
            rules={[{ required: true, message: 'Please select date' }]}
          >
            <DatePicker showTime style={{ width: '100%' }} />
          </Form.Item>

          <Form.Item name="notes" label="Notes">
            <Input.TextArea placeholder="Optional trade notes" rows={2} />
          </Form.Item>
        </Form>
      </Modal>
    </Card>
  );
}
