import { useEffect } from "react";
import {
  Card,
  Form,
  Input,
  Button,
  Select,
  Typography,
  Alert,
  message,
  Divider,
} from "antd";
import { RobotOutlined, SaveOutlined } from "@ant-design/icons";
import { useAiStore } from "../../stores/aiStore";

const { Title, Text, Paragraph } = Typography;
const { TextArea } = Input;

export default function AIPage() {
  const { config, loading, fetchConfig, updateConfig } = useAiStore();
  const [form] = Form.useForm();

  useEffect(() => {
    fetchConfig();
  }, [fetchConfig]);

  useEffect(() => {
    if (config) {
      form.setFieldsValue({
        provider: config.provider,
        api_key: config.api_key,
        model: config.model,
        base_url: config.base_url || "",
        system_prompt: config.system_prompt,
      });
    }
  }, [config, form]);

  const handleSave = async () => {
    try {
      const values = await form.validateFields();
      const success = await updateConfig({
        provider: values.provider,
        api_key: values.api_key,
        model: values.model,
        base_url: values.base_url || null,
        system_prompt: values.system_prompt,
      });
      if (success) {
        message.success("AI 配置已保存");
      }
    } catch (err) {
      message.error("保存失败: " + String(err));
    }
  };

  return (
    <div className="space-y-6">
      <Title level={2}>
        <RobotOutlined /> AI 投资分析（实验性）
      </Title>

      <Alert
        type="info"
        message="实验性功能"
        description="AI 分析功能基于 OpenAI API 或其他兼容 API。API Key 仅本地存储，不会上传。使用前请确保 API Key 有效，并了解相关费用。"
        showIcon
      />

      <Card title="API 配置">
        <Form form={form} layout="vertical" style={{ maxWidth: 600 }}>
          <Form.Item name="provider" label="AI 提供商" rules={[{ required: true }]}>
            <Select
              options={[
                { value: "openai", label: "OpenAI (GPT-4/GPT-3.5)" },
                { value: "custom", label: "自定义 API（OpenAI 兼容）" },
              ]}
            />
          </Form.Item>

          <Form.Item name="model" label="模型" rules={[{ required: true }]}>
            <Select
              options={[
                { value: "gpt-4", label: "GPT-4" },
                { value: "gpt-4-turbo", label: "GPT-4 Turbo" },
                { value: "gpt-3.5-turbo", label: "GPT-3.5 Turbo" },
                { value: "custom", label: "自定义模型" },
              ]}
            />
          </Form.Item>

          <Form.Item
            name="api_key"
            label="API Key"
            rules={[{ required: true, message: "请输入 API Key" }]}
          >
            <Input.Password placeholder="sk-..." />
          </Form.Item>

          <Form.Item name="base_url" label="自定义 API 端点（可选）">
            <Input placeholder="https://api.openai.com/v1（默认）" />
          </Form.Item>

          <Form.Item name="system_prompt" label="系统提示词">
            <TextArea
              rows={4}
              placeholder="你是一位专业的投资顾问..."
            />
          </Form.Item>

          <Form.Item>
            <Button
              type="primary"
              icon={<SaveOutlined />}
              loading={loading}
              onClick={handleSave}
            >
              保存配置
            </Button>
          </Form.Item>
        </Form>
      </Card>

      <Card title="功能说明">
        <Paragraph>
          配置完成后，AI 分析功能可以帮助你：
        </Paragraph>
        <ul className="list-disc list-inside space-y-1">
          <li>分析持仓集中度和风险分布</li>
          <li>基于持仓历史生成季度回顾总结</li>
          <li>提供个性化的投资建议</li>
          <li>分析操作决策的质量和改进方向</li>
        </ul>
        <Divider />
        <Text type="secondary">
          注意：AI 分析仅供参考，不构成投资建议。投资有风险，入市需谨慎。
        </Text>
      </Card>
    </div>
  );
}
