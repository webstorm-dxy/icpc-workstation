import { App, Button, Card, Empty, Space, Spin, Statistic, Tag } from "antd";
import { FilePdfOutlined, PrinterOutlined } from "@ant-design/icons";
import { useQuery } from "@tanstack/react-query";
import { api, errText } from "../lib/api";
import { formatDate } from "../lib/format";
import { MarkdownView } from "../lib/markdown";

type ReportData = {
  period?: { start?: string; end?: string };
  stats?: { submissions?: number; accepted?: number; acceptance_rate?: number };
};

export default function PrintReportPage() {
  const { message } = App.useApp();
  const printId = useQuery({
    queryKey: ["print-report-id"],
    queryFn: api.getPrintReportId,
  });
  const report = useQuery({
    queryKey: ["print-report", printId.data],
    queryFn: () => api.getReport(printId.data!),
    enabled: !!printId.data,
  });

  if (printId.isLoading || report.isLoading) {
    return (
      <div style={{ height: "100vh", display: "grid", placeItems: "center" }}>
        <Spin />
      </div>
    );
  }

  if (!printId.data || !report.data) {
    return (
      <div style={{ height: "100vh", display: "grid", placeItems: "center" }}>
        <Empty description="没有待打印的报告" />
      </div>
    );
  }

  const data = (report.data.data ?? {}) as ReportData;
  const stats = data.stats ?? {};
  return (
    <div className="print-page">
      <div className="print-toolbar">
        <Space>
          <Button
            type="primary"
            icon={<PrinterOutlined />}
            onClick={async () => {
              try {
                await api.printReportWindow();
              } catch (error) {
                message.error(errText(error));
              }
            }}
          >
            打印 / 另存为 PDF
          </Button>
          <Button icon={<FilePdfOutlined />} onClick={() => window.print()}>
            浏览器打印
          </Button>
          <Button
            onClick={async () => {
              try {
                await api.closePrintWindow();
              } catch {
                // ignore
              }
            }}
          >
            关闭
          </Button>
        </Space>
      </div>
      <h1 style={{ fontSize: 30, margin: "0 0 6px" }}>{report.data.title}</h1>
      <div className="muted" style={{ marginBottom: 18 }}>
        统计区间 {data.period?.start ?? "—"} 至 {data.period?.end ?? "—"} · 生成于{" "}
        {formatDate(report.data.createdAt)}
      </div>
      <div
        style={{
          display: "grid",
          gridTemplateColumns: "repeat(3, 1fr)",
          gap: 12,
          marginBottom: 22,
        }}
      >
        <Card size="small">
          <Statistic title="提交数" value={stats.submissions ?? 0} />
        </Card>
        <Card size="small">
          <Statistic title="通过数" value={stats.accepted ?? 0} />
        </Card>
        <Card size="small">
          <Statistic
            title="通过率"
            value={(((stats.acceptance_rate ?? 0) as number) * 100).toFixed(1)}
            suffix="%"
          />
        </Card>
      </div>
      <MarkdownView content={report.data.contentMd} />
      <div className="muted" style={{ marginTop: 28, fontSize: 12 }}>
        由 ICPC 备赛工作站生成 · 数据仅保存在本机
      </div>
    </div>
  );
}
