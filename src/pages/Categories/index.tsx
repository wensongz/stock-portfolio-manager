import { useEffect, useState } from "react";
import {
  Typography,
  Button,
  Table,
  Space,
  Modal,
  Form,
  Input,
  Tag,
  Popconfirm,
  message,
  Badge,
} from "antd";
import { PlusOutlined } from "@ant-design/icons";
import { useCategoryStore } from "../../stores/categoryStore";
import type { Category } from "../../types";

const { Title } = Typography;

export default function CategoriesPage() {
  const { categories, loading, fetchCategories, createCategory, updateCategory, deleteCategory } =
    useCategoryStore();
  const [modalOpen, setModalOpen] = useState(false);
  const [editingCategory, setEditingCategory] = useState<Category | null>(null);
  const [form] = Form.useForm();

  useEffect(() => {
    fetchCategories();
  }, [fetchCategories]);

  const handleSubmit = async (values: { name: string; color: string; icon: string; sort_order?: number }) => {
    try {
      if (editingCategory) {
        await updateCategory({ id: editingCategory.id, ...values });
        message.success("类别更新成功");
      } else {
        await createCategory(values);
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
    form.setFieldsValue(category);
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
        <Title level={2} className="!mb-0">
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
        <Form form={form} layout="vertical" onFinish={handleSubmit}>
          <Form.Item name="icon" label="图标（emoji）"
            rules={[{ required: true, message: "请输入图标" }]}>
            <Input placeholder="如：💰 🚀 🔄" maxLength={2} />
          </Form.Item>
          <Form.Item name="name" label="类别名称"
            rules={[{ required: true, message: "请输入类别名称" }]}>
            <Input placeholder="如：成长股、分红股" />
          </Form.Item>
          <Form.Item name="color" label="颜色（Hex）"
            rules={[{ required: true, message: "请输入颜色" }]}>
            <Input placeholder="#F97316" maxLength={7} />
          </Form.Item>
          <Form.Item name="sort_order" label="排序顺序">
            <Input type="number" placeholder="数字越小越靠前" />
          </Form.Item>
        </Form>
      </Modal>
    </div>
  );
}
