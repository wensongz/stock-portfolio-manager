import { useEffect, useState } from "react";
import {
  Alert,
  Button,
  Descriptions,
  Drawer,
  Empty,
  Input,
  Popconfirm,
  Select,
  Space,
  Table,
  Tag,
  Typography,
  message,
  type TableProps,
} from "antd";
import { DeleteOutlined, ReloadOutlined } from "@ant-design/icons";
import { useAlertHistoryStore } from "../../stores/alertHistoryStore";
import type {
  AlertHistoryMessage,
  AlertHistoryQuery,
} from "../../types/alertHistory";

const { Text, Title } = Typography;

export interface AlertMessageCenterProps {
  open: boolean;
  onClose(): void;
}

export function formatAlertHistoryTime(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return new Intl.DateTimeFormat("zh-CN", {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  }).format(date);
}

export function AlertHistoryMessageDetails({
  message,
}: {
  message: AlertHistoryMessage;
}) {
  if (message.details.length === 0) {
    return <Text type="secondary">这条提醒没有更多详情</Text>;
  }

  return (
    <Descriptions
      bordered
      size="small"
      column={{ xs: 1, sm: 1, md: 2 }}
      items={message.details.map((detail, index) => ({
        key: `${detail.label}-${index}`,
        label: detail.label,
        children: detail.value,
      }))}
    />
  );
}

export function AlertHistoryScope({ message }: { message: AlertHistoryMessage }) {
  const showAccountName = message.accountName
    && !message.scopeName.includes(message.accountName);

  return (
    <Space orientation="vertical" size={0}>
      <Text>{message.scopeName}</Text>
      {showAccountName ? (
        <Text type="secondary">{message.accountName}</Text>
      ) : null}
    </Space>
  );
}

function kindLabel(kind: AlertHistoryMessage["kind"]): string {
  return kind === "PRICE" ? "价格提醒" : "组合提醒";
}

export default function AlertMessageCenter({
  open,
  onClose,
}: AlertMessageCenterProps) {
  const items = useAlertHistoryStore((state) => state.items);
  const total = useAlertHistoryStore((state) => state.total);
  const page = useAlertHistoryStore((state) => state.page);
  const pageSize = useAlertHistoryStore((state) => state.pageSize);
  const kind = useAlertHistoryStore((state) => state.kind);
  const search = useAlertHistoryStore((state) => state.search);
  const loading = useAlertHistoryStore((state) => state.loading);
  const error = useAlertHistoryStore((state) => state.error);
  const deletingId = useAlertHistoryStore((state) => state.deletingId);
  const deleteMessage = useAlertHistoryStore((state) => state.deleteMessage);
  const fetchHistory = useAlertHistoryStore((state) => state.fetchHistory);
  const setKind = useAlertHistoryStore((state) => state.setKind);
  const setSearch = useAlertHistoryStore((state) => state.setSearch);
  const setPagination = useAlertHistoryStore((state) => state.setPagination);
  const [searchDraft, setSearchDraft] = useState(search);
  const [messageApi, messageHolder] = message.useMessage();

  const handleDelete = async (id: string) => {
    try {
      if (await deleteMessage(id)) messageApi.success("消息已删除");
    } catch (error) {
      messageApi.error(`删除失败：${error instanceof Error ? error.message : String(error)}`);
    }
  };

  useEffect(() => {
    if (!open) return;
    void fetchHistory();
  }, [fetchHistory, open]);

  useEffect(() => {
    if (open) setSearchDraft(search);
  }, [open, search]);

  const columns: TableProps<AlertHistoryMessage>["columns"] = [
    {
      title: "触发时间",
      dataIndex: "triggeredAt",
      width: 170,
      render: (value: string) => formatAlertHistoryTime(value),
    },
    {
      title: "类型",
      dataIndex: "kind",
      width: 100,
      render: (value: AlertHistoryMessage["kind"]) => (
        <Tag color={value === "PRICE" ? "blue" : "orange"}>
          {kindLabel(value)}
        </Tag>
      ),
    },
    {
      title: "范围 / 账户",
      width: 130,
      render: (_, message) => <AlertHistoryScope message={message} />,
    },
    {
      title: "提醒内容",
      render: (_, message) => (
        <Space orientation="vertical" size={2}>
          <Text strong>{message.title}</Text>
          <Text type="secondary">{message.message}</Text>
        </Space>
      ),
    },
    {
      title: "操作",
      key: "actions",
      width: 90,
      fixed: "right",
      render: (_, record) => (
        <Popconfirm
          title="删除这条消息？"
          description="删除后无法恢复。"
          okText="删除"
          cancelText="取消"
          okButtonProps={{ danger: true }}
          onConfirm={() => handleDelete(record.id)}
          disabled={loading || deletingId !== null}
        >
          <Button
            danger
            type="text"
            size="small"
            icon={<DeleteOutlined />}
            aria-label={`删除消息：${record.title}`}
            loading={deletingId === record.id}
            disabled={loading || (deletingId !== null && deletingId !== record.id)}
          >
            删除
          </Button>
        </Popconfirm>
      ),
    },
  ];

  const handleKindChange = (value: "ALL" | NonNullable<AlertHistoryQuery["kind"]>) => {
    void setKind(value === "ALL" ? undefined : value);
  };

  return (
    <Drawer
      title="消息中心"
      open={open}
      onClose={onClose}
      size="min(1000px, 96vw)"
      destroyOnHidden
    >
      {messageHolder}
      <Space orientation="vertical" size="middle" style={{ width: "100%" }}>
        <div>
          <Title level={4} style={{ margin: 0 }}>历史提醒</Title>
          <Text type="secondary">按触发时间查看价格和组合提醒记录</Text>
        </div>

        <Space wrap>
          <Input.Search
            allowClear
            value={searchDraft}
            placeholder="搜索标题、内容、范围或账户"
            style={{ width: 320, maxWidth: "100%" }}
            onChange={(event) => {
              setSearchDraft(event.target.value);
            }}
            onSearch={(value) => void setSearch(value)}
          />
          <Select
            aria-label="提醒类型"
            value={kind ?? "ALL"}
            style={{ width: 130 }}
            options={[
              { value: "ALL", label: "全部类型" },
              { value: "PORTFOLIO", label: "组合提醒" },
              { value: "PRICE", label: "价格提醒" },
            ]}
            onChange={handleKindChange}
          />
          <Button
            icon={<ReloadOutlined />}
            loading={loading}
            onClick={() => void fetchHistory()}
          >
            刷新
          </Button>
        </Space>

        {error ? (
          <Alert
            showIcon
            type="error"
            message="历史消息加载失败"
            description={error}
            action={(
              <Button size="small" onClick={() => void fetchHistory()}>
                重试
              </Button>
            )}
          />
        ) : null}

        <Table<AlertHistoryMessage>
          columns={columns}
          dataSource={items}
          loading={loading}
          rowKey="id"
          scroll={{ x: 910 }}
          expandable={{
            expandedRowRender: (message) => (
              <AlertHistoryMessageDetails message={message} />
            ),
            rowExpandable: (message) => message.details.length > 0,
          }}
          locale={{
            emptyText: (
              <Empty
                image={Empty.PRESENTED_IMAGE_SIMPLE}
                description={(
                  <Space orientation="vertical" size={0}>
                    {kind || search ? (
                      <Text>没有符合当前筛选条件的历史消息</Text>
                    ) : (
                      <>
                        <Text>暂无历史消息</Text>
                        <Text type="secondary">
                          后续触发的投资提醒会记录在这里；此前未保存的历史消息无法补回。
                        </Text>
                      </>
                    )}
                  </Space>
                )}
              />
            ),
          }}
          pagination={{
            current: page,
            pageSize,
            total,
            showSizeChanger: true,
            pageSizeOptions: [20, 50, 100],
            showTotal: (count) => `共 ${count} 条`,
            onChange: (nextPage, nextPageSize) => {
              void setPagination(nextPage, nextPageSize);
            },
          }}
        />
      </Space>
    </Drawer>
  );
}
