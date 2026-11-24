import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";
import rehypeKatex from "rehype-katex";
import rehypeSanitize from "rehype-sanitize";
import "katex/dist/katex.min.css";
import { openUrl } from "@tauri-apps/plugin-opener";

export function MarkdownView({
  content,
  className = "",
}: {
  content: string;
  className?: string;
}) {
  return (
    <div className={`markdown ${className}`}>
      <ReactMarkdown
        remarkPlugins={[remarkGfm, remarkMath]}
        rehypePlugins={[rehypeSanitize, [rehypeKatex, { output: "html" }]]}
        components={{
          a: ({ href, children }) => (
            <a
              href={href}
              onClick={(event) => {
                event.preventDefault();
                if (href) {
                  openUrl(href).catch(() => {
                    window.open(href, "_blank");
                  });
                }
              }}
            >
              {children}
            </a>
          ),
          code: ({ className: codeClass, children, ...props }) => (
            <code className={codeClass} {...props}>
              {children}
            </code>
          ),
        }}
      >
        {content || "（暂无内容）"}
      </ReactMarkdown>
    </div>
  );
}
