import { useState, useEffect, useCallback } from 'react';
import {
  Table,
  Button,
  Modal,
  Form,
  Input,
  ColorPicker,
  Space,
  Tag,
  message,
  Popconfirm,
  Card,
  InputNumber,
} from 'antd';
import { PlusOutlined, EditOutlined, DeleteOutlined } from '@ant-design/icons';
import { categoryApi } from '../api';
import type { Category } from '../types';

export default function CategoriesPage() {
  const [categories, setCategories] = useState<Category[]>([]);
  const [loading, setLoading] = useState(false);
  const [modalOpen, setModalOpen] = useState(false);
  const [editingCategory, setEditingCategory] = useState<Category | null>(null);
  const [form] = Form.useForm();

  const loadCategories = useCallback(async () => {
    setLoading(true);
    try {
      const data = await categoryApi.list();
      setCategories(data);
    } catch (e) {
      message.error(`Failed to load categories: ${e}`);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    loadCategories();
  }, [loadCategories]);

  const handleCreate = () => {
    setEditingCategory(null);
    form.resetFields();
    form.setFieldsValue({ color: '#808080' });
    setModalOpen(true);
  };

  const handleEdit = (category: Category) => {
    setEditingCategory(category);
    form.setFieldsValue({
      name: category.name,
      color: category.color,
      icon: category.icon,
      sort_order: category.sort_order,
    });
    setModalOpen(true);
  };

  const handleDelete = async (id: string) => {
    try {
      await categoryApi.delete(id);
      message.success('Category deleted');
      loadCategories();
    } catch (e) {
      message.error(`Failed to delete: ${e}`);
    }
  };

  const handleSubmit = async () => {
    try {
      const values = await form.validateFields();
      const color =
        typeof values.color === 'string'
          ? values.color
          : values.color?.toHexString?.() ?? '#808080';

      if (editingCategory) {
        await categoryApi.update(editingCategory.id, {
          name: values.name,
          color,
          icon: values.icon,
          sort_order: values.sort_order,
        });
        message.success('Category updated');
      } else {
        await categoryApi.create({
          name: values.name,
          color,
          icon: values.icon,
          sort_order: values.sort_order,
        });
        message.success('Category created');
      }
      setModalOpen(false);
      loadCategories();
    } catch (e) {
      if (typeof e === 'string') message.error(e);
    }
  };

  const columns = [
    {
      title: 'Icon',
      dataIndex: 'icon',
      key: 'icon',
      width: 60,
    },
    {
      title: 'Name',
      dataIndex: 'name',
      key: 'name',
    },
    {
      title: 'Color',
      dataIndex: 'color',
      key: 'color',
      render: (color: string) => (
        <Tag color={color} style={{ color: '#fff' }}>
          {color}
        </Tag>
      ),
    },
    {
      title: 'Type',
      dataIndex: 'is_system',
      key: 'is_system',
      render: (isSystem: boolean) =>
        isSystem ? (
          <Tag color="default">System</Tag>
        ) : (
          <Tag color="green">Custom</Tag>
        ),
    },
    {
      title: 'Order',
      dataIndex: 'sort_order',
      key: 'sort_order',
    },
    {
      title: 'Actions',
      key: 'actions',
      render: (_: unknown, record: Category) => (
        <Space>
          <Button
            type="link"
            icon={<EditOutlined />}
            onClick={() => handleEdit(record)}
          >
            Edit
          </Button>
          {!record.is_system && (
            <Popconfirm
              title="Delete this category?"
              onConfirm={() => handleDelete(record.id)}
            >
              <Button type="link" danger icon={<DeleteOutlined />}>
                Delete
              </Button>
            </Popconfirm>
          )}
        </Space>
      ),
    },
  ];

  return (
    <Card
      title="Investment Categories"
      extra={
        <Button type="primary" icon={<PlusOutlined />} onClick={handleCreate}>
          Add Category
        </Button>
      }
    >
      <Table
        columns={columns}
        dataSource={categories}
        rowKey="id"
        loading={loading}
        pagination={false}
      />

      <Modal
        title={editingCategory ? 'Edit Category' : 'Add Category'}
        open={modalOpen}
        onOk={handleSubmit}
        onCancel={() => setModalOpen(false)}
      >
        <Form form={form} layout="vertical">
          <Form.Item
            name="name"
            label="Category Name"
            rules={[
              { required: true, message: 'Please enter category name' },
            ]}
          >
            <Input placeholder="e.g., 科技股, Value" />
          </Form.Item>

          <Form.Item name="color" label="Color">
            <ColorPicker format="hex" />
          </Form.Item>

          <Form.Item name="icon" label="Icon (Emoji)">
            <Input placeholder="e.g., 📈, 🏠" />
          </Form.Item>

          <Form.Item name="sort_order" label="Sort Order">
            <InputNumber min={0} />
          </Form.Item>
        </Form>
      </Modal>
    </Card>
  );
}
