import { RobotOutlined } from "@ant-design/icons";
import { Button } from "antd";
import { useNavigate } from "react-router-dom";
import {
  buildStatisticsAiReviewPrefill,
  type StatisticsAiReviewScope,
} from "./statisticsAiReview";

interface Props {
  scope: StatisticsAiReviewScope;
}

export default function StatisticsAiReviewButton({ scope }: Props) {
  const navigate = useNavigate();

  const handleClick = () => {
    const prefill = buildStatisticsAiReviewPrefill(scope);
    navigate("/ai-assistant", {
      state: {
        prefillPrompt: prefill.prompt,
        prefillActiveSkill: prefill.activeSkill,
        prefillAutoSend: prefill.autoSend,
        prefillToolName: prefill.toolName,
        prefillToolArguments: prefill.toolArguments,
      },
    });
  };

  return (
    <Button
      type="primary"
      size="small"
      icon={<RobotOutlined />}
      onClick={handleClick}
    >
      AI复盘这个组合
    </Button>
  );
}
