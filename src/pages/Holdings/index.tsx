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
} from "antd";
import { PlusOutlined } from "@ant-design/icons";
import { useHoldingStore } from "../../stores/holdingStore";
import { useAccountStore } from "../../stores/accountStore";
import { useCategoryStore } from "../../stores/categoryStore";
import type { Holding, Market, Currency } from "../../types";

const { Title } = Typography;

const marketColors: Record<Market, string> = {
  US: "blue",
  CN: "red",
  HK: "green",
};

export default function HoldingsPage() {
  const { holdings, loading, fetchHoldings, createHolding, updateHolding, deleteHolding } =
    useHoldingStore();
  const { accounts, fetchAccounts } = useAccountStore();
  const { categories, fetchCategories } = useCategoryStore();
  const [modalOpen, setModalOpen] = useState(false);
  const [editingHolding, setEditingHolding] = useState<Holding | null>(null);
  const [form] = Form.useForm();

  useEffect(() => {
    fetchHoldings();
    fetchAccounts();
    fetchCategories();
  }, [fetchHoldings, fetchAccounts, fetchCategories]);

  const handleSubmit = async (values: {
    account_id: string;
    symbol: string;
    name: string;
    market: Market;
    category_id?: string;
    shares: number;
    avg_cost: number;
    currency: Currency;
  }) => {
    try {
      if (editingHolding) {
        await updateHolding({ id: editingHolding.id, ...values });
        message.success("持仓更新成功");
      } else {
        await createHolding(values);
        message.success("持仓创建成功");
      }
      setModalOpen(false);
      form.resetFields();
      setEditingHolding(null);
    } catch (err) {
      message.error(`操作失败: ${err}`);
    }
  };

  const handleEdit = (holding: Holding) => {
    setEditingHolding(holding);
    form.setFieldsValue(holding);
    setModalOpen(true);
  };

  const handleDelete = async (id: string) => {
    try {
      await deleteHolding(id);
      message.success("持仓删除成功");
    } catch (err) {
      message.error(`删除失败: ${err}`);
    }
  };

  const accountMap = Object.fromEntries(accounts.map((a) => [a.id, a.name]));
  const categoryMap = Object.fromEntries(categories.map((c) => [c.id, c]));

  const columns = [
    {
      title: "股票代码",
      dataIndex: "symbol",
      key: "symbol",
      render: (symbol: string, record: Holding) => (
        <Space>
          <Tag color={marketColors[record.market]}>{record.market}</Tag>
          <strong>{symbol}</strong>
        </Space>
      ),
    },
    { title: "股票名称", dataIndex: "name", key: "name" },
    {
      title: "所属账户",
      dataIndex: "account_id",
      key: "account_id",
      render: (id: string) => accountMap[id] || id,
    },
    {
      title: "投资类别",
      dataIndex: "category_id",
      key: "category_id",
      render: (id: string | null) => {
        if (!id) return "—";
        const cat = categoryMap[id];
        return cat ? (
          <Tag color={cat.color}>
            {cat.icon} {cat.name}
          </Tag>
        ) : "—";
      },
    },
    {
      title: "持仓股数",
      dataIndex: "shares",
      key: "shares",
      render: (v: number) => v.toLocaleString(),
    },
    {
      title: "平均成本",
      dataIndex: "avg_cost",
      key: "avg_cost",
      render: (v: number, record: Holding) =>
        `${record.currency} ${v.toFixed(2)}`,
    },
    {
      title: "操作",
      key: "action",
      render: (_: unknown, record: Holding) => (
        <Space>
          <Button type="link" size="small" onClick={() => handleEdit(record)}>
            编辑
          </Button>
          <Popconfirm
            title="确认删除该持仓？"
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
          持仓管理
        </Title>
        <Button
          type="primary"
          icon={<PlusOutlined />}
          onClick={() => {
            setEditingHolding(null);
            form.resetFields();
            setModalOpen(true);
          }}
        >
          新增持仓
        </Button>
      </div>

      <Table
        dataSource={holdings}
        columns={columns}
        rowKey="id"
        loading={loading}
        pagination={{ pageSize: 20 }}
      />

      <Modal
        title={editingHolding ? "编辑持仓" : "新增持仓"}
        open={modalOpen}
        onOk={() => form.submit()}
        onCancel={() => {
          setModalOpen(false);
          setEditingHolding(null);
          form.resetFields();
        }}
        okText="确认"
        cancelText="取消"
        width={600}
      >
        <Form form={form} layout="vertical" onFinish={handleSubmit}>
          <Form.Item
            name="account_id"
            label="所属账户"
            rules={[{ required: true, message: "请选择账户" }]}
          >
            <Select placeholder="选择证券账户">
              {accounts.map((a) => (
                <Select.Option key={a.id} value={a.id}>
                  [{a.market}] {a.name}
                </Select.Option>
              ))}
            </Select>
          </Form.Item>
          <Form.Item
            name="symbol"
            label="股票代码"
            rules={[{ required: true, message: "请输入股票代码" }]}
          >
            <Input placeholder="如：AAPL, 600519.SH, 0700.HK" />
          </Form.Item>
          <Form.Item
            name="name"
            label="股票名称"
            rules={[{ required: true, message: "请输入股票名称" }]}
          >
            <Input placeholder="如：苹果, 贵州茅台, 腾讯控股" />
          </Form.Item>
          <Form.Item
            name="market"
            label="市场"
            rules={[{ required: true, message: "请选择市场" }]}
          >
            <Select placeholder="选择市场">
              <Select.Option value="US">🇺🇸 美股</Select.Option>
              <Select.Option value="CN">🇨🇳 A股</Select.Option>
              <Select.Option value="HK">🇭🇰 港股</Select.Option>
            </Select>
          </Form.Item>
          <Form.Item name="category_id" label="投资类别">
            <Select placeholder="选择投资类别（可选）" allowClear>
              {categories.map((c) => (
                <Select.Option key={c.id} value={c.id}>
                  {c.icon} {c.name}
                </Select.Option>
              ))}
            </Select>
          </Form.Item>
          <Form.Item
            name="shares"
            label="持仓股数"
            rules={[{ required: true, message: "请输入持仓股数" }]}
          >
            <InputNumber min={0} precision={2} style={{ width: "100%" }} placeholder="持有股数" />
          </Form.Item>
          <Form.Item
            name="avg_cost"
            label="平均成本价"
            rules={[{ required: true, message: "请输入平均成本价" }]}
          >
            <InputNumber min={0} precision={4} style={{ width: "100%" }} placeholder="买入均价" />
          </Form.Item>
          <Form.Item
            name="currency"
            label="币种"
            rules={[{ required: true, message: "请选择币种" }]}
          >
            <Select placeholder="选择币种">
              <Select.Option value="USD">USD 美元</Select.Option>
              <Select.Option value="CNY">CNY 人民币</Select.Option>
              <Select.Option value="HKD">HKD 港元</Select.Option>
            </Select>
          </Form.Item>
        </Form>
      </Modal>
    </div>
  );
}
