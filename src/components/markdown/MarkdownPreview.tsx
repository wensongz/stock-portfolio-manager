import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import "./markdown-preview.css";

interface Props {
  source: string;
  className?: string;
}

export default function MarkdownPreview({ source, className = "" }: Props) {
  return (
    <div className={`markdown-preview ${className}`.trim()}>
      <ReactMarkdown remarkPlugins={[remarkGfm]}>{source}</ReactMarkdown>
    </div>
  );
}
