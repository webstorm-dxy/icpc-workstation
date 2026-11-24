import { Alert, Button, Drawer, Space, Spin, Tag } from "antd";
import { useEffect, useRef } from "react";
import { useAppStore } from "../store";

export default function PlanAgentDrawer() {
  const planAgent = useAppStore((s) => s.planAgent);
  const close = useAppStore((s) => s.closePlanAgent);
  const endRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!planAgent?.open) return;
    endRef.current?.scrollIntoView({ behavior: "smooth", block: "end" });
  }, [planAgent?.open, planAgent?.output, planAgent?.phase]);

  if (!planAgent?.open) return null;
  const running =
    planAgent.phase === "start" ||
    planAgent.phase === "model" ||
    planAgent.phase === "output";

  return (
    <Drawer
      open
      width={680}
      placement="right"
      title={
        <Space>
          <span>Agent 训练分析</span>
          <Tag color={planAgent.phase === "error" ? "red" : planAgent.phase === "done" ? "green" : "blue"}>
            {planAgent.phase === "done"
              ? "已完成"
              : planAgent.phase === "error"
                ? "失败"
                : "分析中"}
          </Tag>
        </Space>
      }
      onClose={close}
      footer={
        <div className="flex between center">
          <span className="muted tiny">
            这是 Agent 的原始输出；训练页会展示整理后的诊断与推荐。
          </span>
          <Button onClick={close}>关闭</Button>
        </div>
      }
      className="plan-agent-drawer"
    >
      <Space orientation="vertical" size={10} style={{ width: "100%" }}>
        <div className="flex between center">
          <span>{planAgent.status}</span>
          {running ? <Spin size="small" /> : null}
        </div>
        <Space wrap size={4}>
          {planAgent.provider ? <Tag color="blue">{planAgent.provider}</Tag> : null}
          {planAgent.model ? <Tag>{planAgent.model}</Tag> : null}
        </Space>
        {planAgent.error ? (
          <Alert
            type="error"
            showIcon
            title="Agent 生成失败"
            description={planAgent.error}
          />
        ) : null}
        <div className="plan-agent-output-wrap">
          <pre className="plan-agent-output">
            {planAgent.output ||
              (planAgent.error ? "" : "等待 Agent 输出…")}
          </pre>
          <div ref={endRef} />
        </div>
      </Space>
    </Drawer>
  );
}
