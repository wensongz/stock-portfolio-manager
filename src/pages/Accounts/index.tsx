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
  Tag,
  Popconfirm,
  message,
} from "antd";
import { PlusOutlined } from "@ant-design/icons";
import { useAccountStore } from "../../stores/accountStore";
import type { Account, Market } from "../../types";

const { Title } = Typography;

const marketLabels: Record<Market, string> = {
  US: "🇺🇸 美股",
  CN: "🇨🇳 A股",
  HK: "🇭🇰 港股",
};

const marketColors: Record<Market, string> = {
  US: "blue",
  CN: "red",
  HK: "green",
};

export default function AccountsPage() {
  const { accounts, loading, fetchAccounts, createAccount, updateAccount, deleteAccount } =
    useAccountStore();
  const [modalOpen, setModalOpen] = useState(false);
  const [editingAccount, setEditingAccount] = useState<Account | null>(null);
  const [form] = Form.useForm();

  useEffect(() => {
    fetchAccounts();
  }, [fetchAccounts]);

  const handleSubmit = async (values: { name: string; market: Market; description?: string }) => {
    try {
      if (editingAccount) {
        await updateAccount({ id: editingAccount.id, ...values });
        message.success("账户更新成功");
      } else {
        await createAccount(values);
        message.success("账户创建成功");
      }
      setModalOpen(false);
      form.resetFields();
      setEditingAccount(null);
    } catch (err) {
      message.error(`操作失败: ${err}`);
    }
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
      await deleteAccount(id);
      message.success("账户删除成功");
    } catch (err) {
      message.error(`删除失败: ${err}`);
    }
  };

  const columns = [
    {
      title: "账户名称",
      dataIndex: "name",
      key: "name",
    },
    {
      title: "市场",
      dataIndex: "market",
      key: "market",
      render: (market: Market) => (
        <Tag color={marketColors[market]}>{marketLabels[market]}</Tag>
      ),
    },
    {
      title: "备注",
      dataIndex: "description",
      key: "description",
      render: (desc: string | null) => desc || "—",
    },
    {
      title: "创建时间",
      dataIndex: "created_at",
      key: "created_at",
      render: (date: string) => new Date(date).toLocaleDateString("zh-CN"),
    },
    {
      title: "操作",
      key: "action",
      render: (_: unknown, record: Account) => (
        <Space>
          <Button type="link" size="small" onClick={() => handleEdit(record)}>
            编辑
          </Button>
          <Popconfirm
            title="确认删除该账户？"
            description="删除账户会同时删除相关持仓和交易记录。"
            onConfirm={() => handleDelete(record.id)}
            okText="确认"
            cancelText="取消"
          >
            <Button type="link" size="small" danger>
              删除
            </Button>
          </Popconfirm>
        </Space>
      ),
    },
  ];

  return (
    <div>
      <div className="flex justify-between items-center mb-4">
        <Title level={2} className="!mb-0">
          证券账户管理
        </Title>
        <Button
          type="primary"
          icon={<PlusOutlined />}
          onClick={() => {
            setEditingAccount(null);
            form.resetFields();
            setModalOpen(true);
          }}
        >
          新增账户
        </Button>
      </div>

      <Table
        dataSource={accounts}
        columns={columns}
        rowKey="id"
        loading={loading}
        pagination={{ pageSize: 10 }}
      />

      <Modal
        title={editingAccount ? "编辑账户" : "新增账户"}
        open={modalOpen}
        onOk={() => form.submit()}
        onCancel={() => {
          setModalOpen(false);
          setEditingAccount(null);
          form.resetFields();
        }}
        okText="确认"
        cancelText="取消"
      >
        <Form form={form} layout="vertical" onFinish={handleSubmit}>
          <Form.Item
            name="name"
            label="账户名称"
            rules={[{ required: true, message: "请输入账户名称" }]}
          >
            <Input placeholder="如：Robinhood, 中信证券" />
          </Form.Item>
          <Form.Item
            name="market"
            label="所属市场"
            rules={[{ required: true, message: "请选择市场" }]}
          >
            <Select placeholder="选择市场">
              <Select.Option value="US">🇺🇸 美股 (US)</Select.Option>
              <Select.Option value="CN">🇨🇳 A股 (CN)</Select.Option>
              <Select.Option value="HK">🇭🇰 港股 (HK)</Select.Option>
            </Select>
          </Form.Item>
          <Form.Item name="description" label="备注（可选）">
            <Input.TextArea placeholder="账户备注信息" rows={3} />
          </Form.Item>
        </Form>
      </Modal>
    </div>
  );
}
