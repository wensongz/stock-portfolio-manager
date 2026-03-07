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
} from 'antd';
import { PlusOutlined, EditOutlined, DeleteOutlined } from '@ant-design/icons';
import { accountApi } from '../api';
import type { Account } from '../types';
import { MARKET_LABELS } from '../types';

const MARKET_COLORS: Record<string, string> = {
  US: 'blue',
  CN: 'red',
  HK: 'gold',
};

export default function AccountsPage() {
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [loading, setLoading] = useState(false);
  const [modalOpen, setModalOpen] = useState(false);
  const [editingAccount, setEditingAccount] = useState<Account | null>(null);
  const [form] = Form.useForm();

  const loadAccounts = useCallback(async () => {
    setLoading(true);
    try {
      const data = await accountApi.list();
      setAccounts(data);
    } catch (e) {
      message.error(`Failed to load accounts: ${e}`);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    loadAccounts();
  }, [loadAccounts]);

  const handleCreate = () => {
    setEditingAccount(null);
    form.resetFields();
    setModalOpen(true);
  };

  const handleEdit = (account: Account) => {
    setEditingAccount(account);
    form.setFieldsValue({
      name: account.name,
      market: account.market,
      description: account.description,
    });
    setModalOpen(true);
  };

  const handleDelete = async (id: string) => {
    try {
      await accountApi.delete(id);
      message.success('Account deleted');
      loadAccounts();
    } catch (e) {
      message.error(`Failed to delete: ${e}`);
    }
  };

  const handleSubmit = async () => {
    try {
      const values = await form.validateFields();
      if (editingAccount) {
        await accountApi.update(editingAccount.id, {
          name: values.name,
          description: values.description,
        });
        message.success('Account updated');
      } else {
        await accountApi.create(values);
        message.success('Account created');
      }
      setModalOpen(false);
      loadAccounts();
    } catch (e) {
      if (typeof e === 'string') message.error(e);
    }
  };

  const columns = [
    {
      title: 'Name',
      dataIndex: 'name',
      key: 'name',
    },
    {
      title: 'Market',
      dataIndex: 'market',
      key: 'market',
      render: (market: string) => (
        <Tag color={MARKET_COLORS[market]}>{MARKET_LABELS[market]}</Tag>
      ),
    },
    {
      title: 'Description',
      dataIndex: 'description',
      key: 'description',
    },
    {
      title: 'Actions',
      key: 'actions',
      render: (_: unknown, record: Account) => (
        <Space>
          <Button
            type="link"
            icon={<EditOutlined />}
            onClick={() => handleEdit(record)}
          >
            Edit
          </Button>
          <Popconfirm
            title="Delete this account?"
            description="This will also delete all holdings and transactions in this account."
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
      title="Securities Accounts"
      extra={
        <Button type="primary" icon={<PlusOutlined />} onClick={handleCreate}>
          Add Account
        </Button>
      }
    >
      <Table
        columns={columns}
        dataSource={accounts}
        rowKey="id"
        loading={loading}
        pagination={false}
      />

      <Modal
        title={editingAccount ? 'Edit Account' : 'Add Account'}
        open={modalOpen}
        onOk={handleSubmit}
        onCancel={() => setModalOpen(false)}
      >
        <Form form={form} layout="vertical">
          <Form.Item
            name="name"
            label="Account Name"
            rules={[{ required: true, message: 'Please enter account name' }]}
          >
            <Input placeholder="e.g., Robinhood, 中信证券" />
          </Form.Item>

          {!editingAccount && (
            <Form.Item
              name="market"
              label="Market"
              rules={[{ required: true, message: 'Please select a market' }]}
            >
              <Select
                options={[
                  { label: '🇺🇸 US Market', value: 'US' },
                  { label: '🇨🇳 A-Share (CN)', value: 'CN' },
                  { label: '🇭🇰 HK Market', value: 'HK' },
                ]}
              />
            </Form.Item>
          )}

          <Form.Item name="description" label="Description">
            <Input.TextArea placeholder="Optional notes" rows={2} />
          </Form.Item>
        </Form>
      </Modal>
    </Card>
  );
}
