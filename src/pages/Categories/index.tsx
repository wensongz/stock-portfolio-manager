import { useEffect, useState } from "react";
import {
  Typography,
  Button,
  Table,
  Space,
  Modal,
  Form,
  Input,
  InputNumber,
  ColorPicker,
  Tag,
  Popconfirm,
  message,
  Badge,
} from "antd";
import type { ColorPickerProps } from "antd";
import { PlusOutlined } from "@ant-design/icons";
import { useCategoryStore } from "../../stores/categoryStore";
import type { Category, CreateCategoryPayload } from "../../types";

const { Title } = Typography;

type CategoryFormValues = Omit<CreateCategoryPayload, "sortOrder"> & {
  sortOrder?: number | null;
};

export default function CategoriesPage() {
  const { categories, loading, fetchCategories, createCategory, updateCategory, deleteCategory } =
    useCategoryStore();
  const [modalOpen, setModalOpen] = useState(false);
  const [editingCategory, setEditingCategory] = useState<Category | null>(null);
  const [form] = Form.useForm<CategoryFormValues>();

  useEffect(() => {
    fetchCategories();
  }, [fetchCategories]);

  const handleSubmit = async (values: CategoryFormValues) => {
    const payload: CreateCategoryPayload = { ...values, sortOrder: values.sortOrder ?? undefined };
    try {
      if (editingCategory) {
        await updateCategory({ id: editingCategory.id, ...payload });
        message.success("类别更新成功");
      } else {
        await createCategory(payload);
        message.success("类别创建成功");
      }
      setModalOpen(false);
      form.resetFields();
      setEditingCategory(null);
    } catch (err) {
      message.error(`操作失败: ${err}`);
    }
  };

  const handleEdit = (category: Category) => {
    setEditingCategory(category);
    form.setFieldsValue({
      name: category.name,
      color: category.color,
      icon: category.icon,
      sortOrder: category.sort_order,
    });
    setModalOpen(true);
  };

  const handleDelete = async (id: string) => {
    try {
      await deleteCategory(id);
      message.success("类别删除成功");
    } catch (err) {
      message.error(`删除失败: ${err}`);
    }
  };

  const columns = [
    {
      title: "图标",
      dataIndex: "icon",
      key: "icon",
      width: 60,
      render: (icon: string) => <span style={{ fontSize: 24 }}>{icon}</span>,
    },
    {
      title: "类别名称",
      dataIndex: "name",
      key: "name",
      render: (name: string, record: Category) => (
        <Space>
          <Badge color={record.color} />
          {name}
        </Space>
      ),
    },
    {
      title: "颜色",
      dataIndex: "color",
      key: "color",
      render: (color: string) => (
        <Tag color={color} style={{ fontFamily: "monospace" }}>
          {color}
        </Tag>
      ),
    },
    {
      title: "系统预设",
      dataIndex: "is_system",
      key: "is_system",
      render: (isSystem: boolean) => (isSystem ? <Tag color="blue">系统</Tag> : <Tag>自定义</Tag>),
    },
    {
      title: "排序",
      dataIndex: "sort_order",
      key: "sort_order",
    },
    {
      title: "操作",
      key: "action",
      render: (_: unknown, record: Category) => (
        <Space>
          <Button type="link" size="small" onClick={() => handleEdit(record)}>
            编辑
          </Button>
          {!record.is_system && (
            <Popconfirm
              title="确认删除该类别？"
              onConfirm={() => handleDelete(record.id)}
              okText="确认"
              cancelText="取消"
            >
              <Button type="link" size="small" danger>
                删除
              </Button>
            </Popconfirm>
          )}
        </Space>
      ),
    },
  ];

  return (
    <div>
      <div className="flex justify-between items-center mb-4">
        <Title level={4} className="!mb-0">
          投资类别管理
        </Title>
        <Button
          type="primary"
          icon={<PlusOutlined />}
          onClick={() => {
            setEditingCategory(null);
            form.resetFields();
            setModalOpen(true);
          }}
        >
          新增类别
        </Button>
      </div>

      <Table
        dataSource={categories}
        columns={columns}
        rowKey="id"
        loading={loading}
        pagination={false}
      />

      <Modal
        title={editingCategory ? "编辑类别" : "新增类别"}
        open={modalOpen}
        onOk={() => form.submit()}
        onCancel={() => {
          setModalOpen(false);
          setEditingCategory(null);
          form.resetFields();
        }}
        okText="确认"
        cancelText="取消"
      >
        <Form form={form} layout="vertical" onFinish={handleSubmit}
          initialValues={{ color: "#F97316" }}>
          <Form.Item name="icon" label="图标（emoji）"
            rules={[{ required: true, message: "请输入图标" }]}>
            <Input placeholder="如：💰 🚀 🔄" maxLength={2} />
          </Form.Item>
          <Form.Item name="name" label="类别名称"
            rules={[{ required: true, message: "请输入类别名称" }]}>
            <Input placeholder="如：成长股、分红股" />
          </Form.Item>
          <Form.Item name="color" label="颜色"
            rules={[{ required: true, message: "请选择颜色" }]}
            getValueFromEvent={(color: Parameters<NonNullable<ColorPickerProps["onChange"]>>[0]) => color.toHexString()}>
            <ColorPicker
              format="hex"
              placement="right"
              disabledAlpha
              showText={() => "点击选择颜色"}
              presets={[{
                label: "常用颜色",
                colors: [
                  "#F97316", "#FF8844", "#EF4444", "#F59E0B",
                  "#EAB308", "#22C55E", "#14B8A6", "#06B6D4",
                  "#3B82F6", "#8B5CF6", "#EC4899", "#64748B",
                ],
              }]}
            />
          </Form.Item>
          <Form.Item name="sortOrder" label="排序顺序"
            extra="数字越小越靠前，留空默认 100"
            rules={[{
              type: "integer",
              min: -2147483648,
              max: 2147483647,
              message: "请输入 -2147483648 到 2147483647 之间的整数",
            }]}>
            <InputNumber precision={0} step={1} style={{ width: "100%" }} placeholder="例如：6" />
          </Form.Item>
        </Form>
      </Modal>
    </div>
  );
}
