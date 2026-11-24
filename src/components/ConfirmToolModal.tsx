import { App, Alert, Button, Modal, Space, Typography } from "antd";
import { useState } from "react";
import { api, errText } from "../lib/api";
import { useAppStore } from "../store";

const TOOL_NAMES: Record<string, string> = {
  generate_training_plan: "生成训练计划",
  refresh_training_plan: "刷新训练计划",
  sync_platform: "同步 OJ 数据",
  run_samples: "本地运行样例",
  submit_draft: "填写 OJ 提交页面",
  generate_report: "生成训练报告",
};

export default function ConfirmToolModal() {
  const { message } = App.useApp();
  const confirmation = useAppStore((s) => s.confirmation);
  const setConfirmation = useAppStore((s) => s.setConfirmation);
  const [loading, setLoading] = useState(false);

  const answer = async (approved: boolean) => {
    if (!confirmation) return;
    setLoading(true);
    try {
      await api.confirmTool(confirmation.requestId, approved);
    } catch (error) {
      message.error(errText(error));
    } finally {
      setLoading(false);
      setConfirmation(null);
    }
  };

  const isSubmit = confirmation?.tool === "submit_draft";
  return (
    <Modal
      open={!!confirmation}
      title={confirmation ? `确认：${TOOL_NAMES[confirmation.tool] ?? confirmation.tool}` : ""}
      onCancel={() => answer(false)}
      footer={
        <Space>
          <Button onClick={() => answer(false)} disabled={loading}>
            拒绝
          </Button>
          <Button type="primary" loading={loading} onClick={() => answer(true)}>
            允许
          </Button>
        </Space>
      }
      width={560}
    >
      <Typography.Paragraph>{confirmation?.summary}</Typography.Paragraph>
      {isSubmit ? (
        <Alert
          type="warning"
          showIcon
          style={{ marginBottom: 12 }}
          title="这是会与 OJ 交互的操作"
          description="应用只会把代码填入提交页面，不会自动点击提交按钮。请你在内置浏览器中确认代码与语言。"
        />
      ) : null}
      <details>
        <summary className="muted tiny">查看参数</summary>
        <pre
          className="mono"
          style={{
            maxHeight: 220,
            overflow: "auto",
            background: "#080c11",
            border: "1px solid #1b2634",
            padding: 10,
            borderRadius: 8,
            fontSize: 11,
          }}
        >
          {JSON.stringify(confirmation?.args ?? {}, null, 2)}
        </pre>
      </details>
    </Modal>
  );
}
