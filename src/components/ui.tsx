import type { ReactNode } from "react";
import { Tag, Typography } from "antd";
import { verdictColor, verdictLabel } from "../lib/format";

export function PageHeader({
  title,
  subtitle,
  extra,
}: {
  title: ReactNode;
  subtitle?: ReactNode;
  extra?: ReactNode;
}) {
  return (
    <div className="page-header">
      <div>
        <h1 className="page-title">{title}</h1>
        {subtitle ? <div className="page-subtitle">{subtitle}</div> : null}
      </div>
      {extra ? <div>{extra}</div> : null}
    </div>
  );
}

export function VerdictTag({ verdict }: { verdict: string }) {
  return <Tag color={verdictColor(verdict)}>{verdictLabel(verdict)}</Tag>;
}

export function PlatformTag({ platform }: { platform: string }) {
  const color =
    platform === "codeforces" ? "blue" : platform === "atcoder" ? "purple" : "gold";
  const label =
    platform === "codeforces" ? "Codeforces" : platform === "atcoder" ? "AtCoder" : "洛谷";
  return <Tag color={color}>{label}</Tag>;
}

export function EmptyState({
  title,
  description,
  action,
}: {
  title: string;
  description?: string;
  action?: ReactNode;
}) {
  return (
    <div
      className="panel"
      style={{
        padding: "44px 24px",
        textAlign: "center",
        display: "grid",
        gap: 10,
        placeItems: "center",
      }}
    >
      <Typography.Title level={4} style={{ margin: 0 }}>
        {title}
      </Typography.Title>
      {description ? (
        <Typography.Paragraph className="muted" style={{ maxWidth: 560, margin: 0 }}>
          {description}
        </Typography.Paragraph>
      ) : null}
      {action}
    </div>
  );
}
