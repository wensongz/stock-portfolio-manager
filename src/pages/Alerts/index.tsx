import { useEffect, useState } from "react";
import {
  Card,
  Table,
  Button,
  Modal,
  Form,
  Select,
  InputNumber,
  Switch,
  Space,
  Tag,
  Typography,
  message,
  Popconfirm,
} from "antd";
import {
  PlusOutlined,
  DeleteOutlined,
  BellOutlined,
} from "@ant-design/icons";
import { useAlertStore } from "../../stores/alertStore";
import { useHoldingStore } from "../../stores/holdingStore";
import type { AlertType, PriceAlert } from "../../types";

const { Title, Text } = Typography;

const ALERT_TYPE_LABELS: Record<AlertType, string> = {
  PRICE_ABOVE: "价格超过",
  PRICE_BELOW: "价格低于",
  CHANGE_ABOVE: "涨幅超过%",
  CHANGE_BELOW: "跌幅超过%",
  PNL_ABOVE: "盈亏超过%",
  PNL_BELOW: "盈亏低于%",
};

const ALERT_TYPE_COLOR: Record<AlertType, string> = {
  PRICE_ABOVE: "green",
  PRICE_BELOW: "red",
  CHANGE_ABOVE: "green",
  CHANGE_BELOW: "red",
  PNL_ABOVE: "blue",
  PNL_BELOW: "orange",
};

export default function AlertsPage() {
  const { alerts, fetchAlerts, createAlert, updateAlert, deleteAlert, loading } =
    useAlertStore();
  const { holdings, fetchHoldings } = useHoldingStore();
  const [modalVisible, setModalVisible] = useState(false);
  const [form] = Form.useForm();

  useEffect(() => {
    fetchAlerts();
    fetchHoldings();
  }, [fetchAlerts, fetchHoldings]);

  const handleCreate = async () => {
    try {
      const values = await form.validateFields();
      const holding = holdings.find((h) => h.id === values.holding_id);
      if (!holding) return;

      await createAlert(
        holding.id,
        holding.symbol,
        holding.name,
        holding.market,
        values.alert_type,
        values.threshold
      );
      message.success("提醒已创建");
      setModalVisible(false);
      form.resetFields();
    } catch (err) {
      message.error("创建失败: " + String(err));
    }
  };

  const columns = [
    {
      title: "股票",
      dataIndex: "symbol",
      key: "symbol",
      render: (_: string, r: PriceAlert) => (
        <Space>
          <Text strong>{r.symbol}</Text>
          <Text type="secondary">{r.name}</Text>
          <Tag>{r.market}</Tag>
        </Space>
      ),
    },
    {
      title: "提醒类型",
      dataIndex: "alert_type",
      key: "alert_type",
      render: (v: AlertType) => (
        <Tag color={ALERT_TYPE_COLOR[v]}>{ALERT_TYPE_LABELS[v]}</Tag>
      ),
    },
    {
      title: "阈值",
      dataIndex: "threshold",
      key: "threshold",
      render: (v: number) => v.toFixed(2),
    },
    {
      title: "状态",
      key: "status",
      render: (_: unknown, r: PriceAlert) =>
        r.is_triggered ? (
          <Tag color="red">已触发</Tag>
        ) : r.is_active ? (
          <Tag color="green">活跃</Tag>
        ) : (
          <Tag color="default">已停用</Tag>
        ),
    },
    {
      title: "启用",
      key: "is_active",
      render: (_: unknown, r: PriceAlert) => (
        <Switch
          checked={r.is_active}
          onChange={(v) => updateAlert(r.id, v)}
          disabled={r.is_triggered}
        />
      ),
    },
    {
      title: "操作",
      key: "action",
      render: (_: unknown, r: PriceAlert) => (
        <Popconfirm
          title="确认删除此提醒？"
          onConfirm={() => deleteAlert(r.id)}
        >
          <Button danger size="small" icon={<DeleteOutlined />} />
        </Popconfirm>
      ),
    },
  ];

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <Title level={2}>
          <BellOutlined /> 价格提醒
        </Title>
        <Button
          type="primary"
          icon={<PlusOutlined />}
          onClick={() => setModalVisible(true)}
        >
          新建提醒
        </Button>
      </div>

      <Card>
        <Table
          dataSource={alerts}
          columns={columns}
          rowKey="id"
          loading={loading}
          pagination={{ pageSize: 20 }}
        />
      </Card>

      <Modal
        title="新建价格提醒"
        open={modalVisible}
        onOk={handleCreate}
        onCancel={() => { setModalVisible(false); form.resetFields(); }}
        confirmLoading={loading}
      >
        <Form form={form} layout="vertical">
          <Form.Item name="holding_id" label="选择持仓" rules={[{ required: true }]}>
            <Select
              showSearch
              placeholder="搜索股票"
              optionFilterProp="label"
              options={holdings.map((h) => ({
                value: h.id,
                label: `${h.symbol} ${h.name} (${h.market})`,
              }))}
            />
          </Form.Item>
          <Form.Item name="alert_type" label="提醒类型" rules={[{ required: true }]}>
            <Select
              options={Object.entries(ALERT_TYPE_LABELS).map(([value, label]) => ({
                value,
                label,
              }))}
            />
          </Form.Item>
          <Form.Item name="threshold" label="阈值" rules={[{ required: true }]}>
            <InputNumber style={{ width: "100%" }} placeholder="输入阈值" />
          </Form.Item>
        </Form>
      </Modal>
    </div>
  );
}
