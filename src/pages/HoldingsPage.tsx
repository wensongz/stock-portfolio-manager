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
} from 'antd';
import { PlusOutlined, EditOutlined, DeleteOutlined } from '@ant-design/icons';
import { holdingApi, accountApi, categoryApi } from '../api';
import type { Holding, Account, Category } from '../types';
import { MARKET_LABELS, CURRENCY_MAP } from '../types';

export default function HoldingsPage() {
  const [holdings, setHoldings] = useState<Holding[]>([]);
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [categories, setCategories] = useState<Category[]>([]);
  const [loading, setLoading] = useState(false);
  const [modalOpen, setModalOpen] = useState(false);
  const [editingHolding, setEditingHolding] = useState<Holding | null>(null);
  const [filterAccountId, setFilterAccountId] = useState<string | undefined>();
  const [form] = Form.useForm();

  const loadData = useCallback(async () => {
    setLoading(true);
    try {
      const [h, a, c] = await Promise.all([
        holdingApi.list(filterAccountId),
        accountApi.list(),
        categoryApi.list(),
      ]);
      setHoldings(h);
      setAccounts(a);
      setCategories(c);
    } catch (e) {
      message.error(`Failed to load data: ${e}`);
    } finally {
      setLoading(false);
    }
  }, [filterAccountId]);

  useEffect(() => {
    loadData();
  }, [loadData]);

  const handleCreate = () => {
    setEditingHolding(null);
    form.resetFields();
    setModalOpen(true);
  };

  const handleEdit = (holding: Holding) => {
    setEditingHolding(holding);
    form.setFieldsValue({
      name: holding.name,
      category_id: holding.category_id,
      shares: holding.shares,
      avg_cost: holding.avg_cost,
    });
    setModalOpen(true);
  };

  const handleDelete = async (id: string) => {
    try {
      await holdingApi.delete(id);
      message.success('Holding deleted');
      loadData();
    } catch (e) {
      message.error(`Failed to delete: ${e}`);
    }
  };

  const handleSubmit = async () => {
    try {
      const values = await form.validateFields();
      if (editingHolding) {
        await holdingApi.update(editingHolding.id, {
          name: values.name,
          category_id: values.category_id,
          shares: values.shares,
          avg_cost: values.avg_cost,
        });
        message.success('Holding updated');
      } else {
        const account = accounts.find((a) => a.id === values.account_id);
        if (!account) return;
        await holdingApi.create({
          account_id: values.account_id,
          symbol: values.symbol,
          name: values.name,
          market: account.market,
          category_id: values.category_id,
          shares: values.shares,
          avg_cost: values.avg_cost,
          currency: CURRENCY_MAP[account.market] as 'USD' | 'CNY' | 'HKD',
        });
        message.success('Holding created');
      }
      setModalOpen(false);
      loadData();
    } catch (e) {
      if (typeof e === 'string') message.error(e);
    }
  };

  const getCategoryName = (categoryId: string | null) => {
    if (!categoryId) return '-';
    const cat = categories.find((c) => c.id === categoryId);
    return cat ? `${cat.icon} ${cat.name}` : categoryId;
  };

  const getAccountName = (accountId: string) => {
    const acc = accounts.find((a) => a.id === accountId);
    return acc ? acc.name : accountId;
  };

  const columns = [
    {
      title: 'Symbol',
      dataIndex: 'symbol',
      key: 'symbol',
      render: (symbol: string) => <strong>{symbol}</strong>,
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
      render: (market: string) => (
        <Tag>{MARKET_LABELS[market]}</Tag>
      ),
    },
    {
      title: 'Category',
      dataIndex: 'category_id',
      key: 'category_id',
      render: (id: string | null) => getCategoryName(id),
    },
    {
      title: 'Shares',
      dataIndex: 'shares',
      key: 'shares',
      align: 'right' as const,
      render: (v: number) => v.toLocaleString(),
    },
    {
      title: 'Avg Cost',
      dataIndex: 'avg_cost',
      key: 'avg_cost',
      align: 'right' as const,
      render: (v: number) => v.toFixed(2),
    },
    {
      title: 'Currency',
      dataIndex: 'currency',
      key: 'currency',
    },
    {
      title: 'Actions',
      key: 'actions',
      render: (_: unknown, record: Holding) => (
        <Space>
          <Button type="link" icon={<EditOutlined />} onClick={() => handleEdit(record)}>
            Edit
          </Button>
          <Popconfirm
            title="Delete this holding?"
            onConfirm={() => handleDelete(record.id)}
          >
            <Button type="link" danger icon={<DeleteOutlined />}>
              Delete
            </Button>
          </Popconfirm>
        </Space>
      ),
    },
  ];

  return (
    <Card
      title="Holdings"
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
          <Button type="primary" icon={<PlusOutlined />} onClick={handleCreate}>
            Add Holding
          </Button>
        </Space>
      }
    >
      <Table
        columns={columns}
        dataSource={holdings}
        rowKey="id"
        loading={loading}
        pagination={false}
        scroll={{ x: 900 }}
      />

      <Modal
        title={editingHolding ? 'Edit Holding' : 'Add Holding'}
        open={modalOpen}
        onOk={handleSubmit}
        onCancel={() => setModalOpen(false)}
        width={520}
      >
        <Form form={form} layout="vertical">
          {!editingHolding && (
            <>
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
                name="symbol"
                label="Stock Symbol"
                rules={[{ required: true, message: 'Please enter symbol' }]}
              >
                <Input placeholder="e.g., AAPL, 600519.SH" />
              </Form.Item>
            </>
          )}

          <Form.Item
            name="name"
            label="Stock Name"
            rules={[{ required: true, message: 'Please enter stock name' }]}
          >
            <Input placeholder="e.g., Apple Inc., 贵州茅台" />
          </Form.Item>

          <Form.Item name="category_id" label="Category">
            <Select
              allowClear
              options={categories.map((c) => ({
                label: `${c.icon} ${c.name}`,
                value: c.id,
              }))}
            />
          </Form.Item>

          <Form.Item
            name="shares"
            label="Shares"
            rules={[{ required: true, message: 'Please enter shares' }]}
          >
            <InputNumber min={0} style={{ width: '100%' }} />
          </Form.Item>

          <Form.Item
            name="avg_cost"
            label="Average Cost"
            rules={[{ required: true, message: 'Please enter average cost' }]}
          >
            <InputNumber min={0} step={0.01} style={{ width: '100%' }} />
          </Form.Item>
        </Form>
      </Modal>
    </Card>
  );
}
