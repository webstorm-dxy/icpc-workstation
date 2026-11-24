import {
  App,
  Button,
  Card,
  DatePicker,
  Empty,
  Modal,
  Select,
  Space,
  Statistic,
  Tag,
  Typography,
} from "antd";
import {
  DownloadOutlined,
  FilePdfOutlined,
  FileTextOutlined,
  PlusOutlined,
  PrinterOutlined,
} from "@ant-design/icons";
import { useEffect, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { save } from "@tauri-apps/plugin-dialog";
import dayjs, { type Dayjs } from "dayjs";
import { api, errText } from "../lib/api";
import { formatDate } from "../lib/format";
import { MarkdownView } from "../lib/markdown";
import { buildReportHtml } from "../lib/reportExport";
import { EmptyState, PageHeader } from "../components/ui";

const KIND_LABELS: Record<string, string> = {
  weekly: "周报",
  monthly: "月报",
  stage: "阶段总结",
  contest: "比赛复盘",
};

type ReportData = {
  stats?: { submissions?: number; accepted?: number; acceptance_rate?: number };
  skill_tags?: Array<{ tag: string; rating: number }>;
  accounts?: Array<{ platform: string; rating?: number; solved?: number }>;
};

function dataStats(report: { data: Record<string, unknown> }): ReportData {
  return report.data as ReportData;
}

export default function ReportsPage() {
  const { message } = App.useApp();
  const queryClient = useQueryClient();
  const reports = useQuery({ queryKey: ["reports"], queryFn: api.listReports });
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [generateOpen, setGenerateOpen] = useState(false);
  const [kind, setKind] = useState("weekly");
  const [range, setRange] = useState<[Dayjs, Dayjs]>([
    dayjs().subtract(7, "day"),
    dayjs(),
  ]);
  const [generating, setGenerating] = useState(false);

  useEffect(() => {
    if (!selectedId && reports.data?.length) {
      setSelectedId(reports.data[0].id);
    }
  }, [reports.data, selectedId]);

  const report = useQuery({
    queryKey: ["report", selectedId],
    queryFn: () => api.getReport(selectedId!),
    enabled: !!selectedId,
  });

  const generate = async () => {
    setGenerating(true);
    message.loading({ content: "正在生成报告…", key: "report" });
    try {
      const created = await api.generateReport({
        kind,
        periodStart: range[0].format("YYYY-MM-DD"),
        periodEnd: range[1].format("YYYY-MM-DD"),
      });
      await queryClient.invalidateQueries({ queryKey: ["reports"] });
      setSelectedId(created.id);
      setGenerateOpen(false);
      message.success({ content: "报告已生成", key: "report" });
    } catch (error) {
      message.error({ content: errText(error), key: "report" });
    } finally {
      setGenerating(false);
    }
  };

  const exportMd = async () => {
    if (!report.data) return;
    const path = await save({
      defaultPath: `${report.data.title.replaceAll("/", "-")}.md`,
      filters: [{ name: "Markdown", extensions: ["md"] }],
    });
    if (!path) return;
    try {
      const result = await api.exportReport({
        reportId: report.data.id,
        format: "md",
        path,
      });
      message.success(`已导出到 ${result}`);
    } catch (error) {
      message.error(errText(error));
    }
  };

  const exportHtml = async () => {
    if (!report.data) return;
    const path = await save({
      defaultPath: `${report.data.title.replaceAll("/", "-")}.html`,
      filters: [{ name: "HTML", extensions: ["html"] }],
    });
    if (!path) return;
    try {
      const html = await buildReportHtml(report.data);
      const result = await api.exportReport({
        reportId: report.data.id,
        format: "html",
        content: html,
        path,
      });
      message.success(`已导出到 ${result}`);
    } catch (error) {
      message.error(errText(error));
    }
  };

  const exportPdf = async () => {
    if (!report.data) return;
    try {
      await api.exportReport({ reportId: report.data.id, format: "pdf" });
      message.info("已打开打印窗口，请在系统对话框中选择“存储为 PDF”");
    } catch (error) {
      message.error(errText(error));
    }
  };

  const stats = report.data ? dataStats(report.data) : null;

  return (
    <div className="page" style={{ height: "100%", overflow: "hidden" }}>
      <PageHeader
        title="报告"
        subtitle="由 Agent 汇总 rating、提交与代码分析生成；支持 Markdown、HTML 与 PDF。"
        extra={
          <Button type="primary" icon={<PlusOutlined />} onClick={() => setGenerateOpen(true)}>
            生成报告
          </Button>
        }
      />
      <div className="report-layout" style={{ height: "calc(100% - 86px)" }}>
        <div className="report-list">
          <Space orientation="vertical" style={{ width: "100%" }} size={8}>
            {reports.data?.length ? (
              reports.data.map((item) => (
                <Card
                  key={item.id}
                  size="small"
                  hoverable
                  onClick={() => setSelectedId(item.id)}
                  style={{
                    borderColor:
                      selectedId === item.id ? "rgba(34,211,238,0.45)" : undefined,
                    background: selectedId === item.id ? "#0d1a21" : "#0f151d",
                  }}
                >
                  <div className="flex between center gap-8">
                    <Tag color="cyan">{KIND_LABELS[item.kind] ?? item.kind}</Tag>
                    <span className="muted tiny">{formatDate(item.createdAt, false)}</span>
                  </div>
                  <div style={{ marginTop: 6, fontWeight: 600, fontSize: 13 }}>
                    {item.title}
                  </div>
                </Card>
              ))
            ) : (
              <Empty description="还没有报告" />
            )}
          </Space>
        </div>
        <div className="report-view">
          {!report.data ? (
            <EmptyState
              title="选择或生成一份报告"
              description="报告会包含阶段统计、标签能力、提交明细与代码分析摘要。"
              action={
                <Button type="primary" icon={<PlusOutlined />} onClick={() => setGenerateOpen(true)}>
                  生成第一份报告
                </Button>
              }
            />
          ) : (
            <>
              <Card className="panel mb-14" size="small">
                <div className="flex between center wrap gap-12">
                  <div>
                    <Typography.Title level={4} style={{ margin: 0 }}>
                      {report.data.title}
                    </Typography.Title>
                    <div className="muted tiny mt-8">
                      统计区间 {report.data.periodStart ?? "—"} 至{" "}
                      {report.data.periodEnd ?? "—"} · 生成于{" "}
                      {formatDate(report.data.createdAt)}
                    </div>
                  </div>
                  <Space wrap>
                    <Button icon={<FileTextOutlined />} onClick={exportMd}>
                      Markdown
                    </Button>
                    <Button icon={<DownloadOutlined />} onClick={exportHtml}>
                      HTML
                    </Button>
                    <Button icon={<FilePdfOutlined />} onClick={exportPdf}>
                      PDF / 打印
                    </Button>
                    <Button
                      icon={<PrinterOutlined />}
                      onClick={async () => {
                        try {
                          await api.exportReport({
                            reportId: report.data!.id,
                            format: "pdf",
                          });
                        } catch (error) {
                          message.error(errText(error));
                        }
                      }}
                    >
                      打印
                    </Button>
                  </Space>
                </div>
              </Card>
              {stats?.stats ? (
                <div className="stat-grid mb-14">
                  <Card className="panel" size="small">
                    <Statistic title="提交数" value={stats.stats.submissions ?? 0} />
                  </Card>
                  <Card className="panel" size="small">
                    <Statistic title="通过数" value={stats.stats.accepted ?? 0} />
                  </Card>
                  <Card className="panel" size="small">
                    <Statistic
                      title="通过率"
                      value={(((stats.stats.acceptance_rate ?? 0) as number) * 100).toFixed(1)}
                      suffix="%"
                    />
                  </Card>
                  <Card className="panel" size="small">
                    <Statistic
                      title="标签数"
                      value={stats.skill_tags?.length ?? 0}
                    />
                  </Card>
                </div>
              ) : null}
              <Card className="panel" size="small">
                <MarkdownView content={report.data.contentMd} />
              </Card>
            </>
          )}
        </div>
      </div>

      <Modal
        open={generateOpen}
        title="生成训练报告"
        onCancel={() => setGenerateOpen(false)}
        onOk={generate}
        confirmLoading={generating}
        okText="生成"
        cancelText="取消"
      >
        <Space orientation="vertical" style={{ width: "100%" }}>
          <div>
            <div className="muted tiny mb-8">报告类型</div>
            <Select
              value={kind}
              onChange={setKind}
              style={{ width: "100%" }}
              options={Object.entries(KIND_LABELS).map(([value, label]) => ({
                value,
                label,
              }))}
            />
          </div>
          <div>
            <div className="muted tiny mb-8">统计区间</div>
            <DatePicker.RangePicker
              value={range}
              onChange={(value) => {
                if (value?.[0] && value[1]) {
                  setRange([value[0], value[1]]);
                }
              }}
              style={{ width: "100%" }}
            />
          </div>
          <div className="muted tiny">
            报告由当前配置的报告角色模型生成；未配置模型时使用本地模板。
          </div>
        </Space>
      </Modal>
    </div>
  );
}
