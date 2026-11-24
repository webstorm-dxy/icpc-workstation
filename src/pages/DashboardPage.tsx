import {
  App,
  Button,
  Card,
  Empty,
  Progress,
  Space,
  Statistic,
  Table,
  Tag,
  Tooltip,
} from "antd";
import {
  CloudSyncOutlined,
  FireOutlined,
  ReloadOutlined,
  RiseOutlined,
  TrophyOutlined,
} from "@ant-design/icons";
import { useMemo } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "react-router-dom";
import { api, errText } from "../lib/api";
import { EChart, ratingChartOption, tagBarOption } from "../lib/chart";
import { formatDate, platformName, verdictColor, verdictLabel } from "../lib/format";
import { useAppStore } from "../store";
import { EmptyState, PageHeader, PlatformTag, VerdictTag } from "../components/ui";
import type { SubmissionRow } from "../types";

export default function DashboardPage() {
  const { message } = App.useApp();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const sync = useAppStore((s) => s.sync);
  const dashboard = useQuery({ queryKey: ["dashboard"], queryFn: api.getDashboard });
  const platforms = useQuery({ queryKey: ["platforms"], queryFn: api.listPlatforms });

  const data = dashboard.data;
  const ratingOption = useMemo(
    () => ratingChartOption(data?.ratingPoints ?? []),
    [data?.ratingPoints],
  );
  const tagOption = useMemo(() => tagBarOption(data?.tagStats ?? []), [data?.tagStats]);
  const maxRating = Math.max(0, ...(data?.accounts.map((a) => a.rating) ?? []));
  const plan = data?.activePlan;
  const planDone = plan?.items.filter((item) => item.status === "done").length ?? 0;

  const startSync = async (platform: string) => {
    try {
      await api.startSync(platform);
      message.info(`已开始同步 ${platformName(platform)}`);
    } catch (error) {
      message.error(errText(error));
    }
  };

  const seed = async () => {
    try {
      message.loading({ content: "正在写入演示数据…", key: "seed" });
      await api.seedDemoData();
      message.success({ content: "演示数据已就绪", key: "seed" });
      await queryClient.invalidateQueries();
    } catch (error) {
      message.error({ content: errText(error), key: "seed" });
    }
  };

  const columns: NonNullable<React.ComponentProps<typeof Table<SubmissionRow>>["columns"]> = [
    {
      title: "题目",
      dataIndex: "problemTitle",
      render: (_, row) => (
        <a onClick={() => navigate(`/problem/${row.problemId}`)}>{row.problemTitle}</a>
      ),
    },
    {
      title: "平台",
      dataIndex: "platform",
      width: 110,
      render: (value: string) => <PlatformTag platform={value} />,
    },
    {
      title: "结果",
      dataIndex: "verdict",
      width: 100,
      render: (value: string) => <VerdictTag verdict={value} />,
    },
    {
      title: "用时",
      dataIndex: "timeMs",
      width: 90,
      render: (value: number | null) => (value ? `${value} ms` : "—"),
    },
    {
      title: "时间",
      dataIndex: "submittedAt",
      width: 150,
      render: (value: string) => <span className="muted">{formatDate(value)}</span>,
    },
    {
      title: "分析",
      dataIndex: "analysisStatus",
      width: 90,
      render: (value: string) => (
        <Tag color={value === "done" ? "green" : value === "failed" ? "red" : "default"}>
          {value === "done" ? "已分析" : value === "pending" ? "待分析" : value}
        </Tag>
      ),
    },
  ];

  return (
    <div className="page">
      <PageHeader
        title="数据面板"
        subtitle="rating、提交与标签能力都来自本地同步；点击同步会重新执行平台读取流程。"
        extra={
          <Space wrap>
            <Button icon={<ReloadOutlined />} onClick={() => dashboard.refetch()}>
              刷新
            </Button>
            {(platforms.data ?? []).map((platform) => (
              <Button
                key={platform.id}
                icon={<CloudSyncOutlined />}
                onClick={() => startSync(platform.id)}
                loading={sync[platform.id]?.status === "running"}
              >
                同步 {platform.name}
              </Button>
            ))}
          </Space>
        }
      />

      {Object.entries(sync).some(([, progress]) => progress.status === "running") ? (
        <Card size="small" style={{ marginBottom: 14 }}>
          <Space orientation="vertical" style={{ width: "100%" }}>
            {Object.entries(sync)
              .filter(([, progress]) => progress.status === "running")
              .map(([platform, progress]) => (
                <div key={platform}>
                  <div className="flex between center">
                    <span>
                      {platformName(platform)} · {progress.stage}
                    </span>
                    <span className="muted tiny">{Math.round(progress.percent)}%</span>
                  </div>
                  <Progress
                    percent={Math.round(progress.percent)}
                    size="small"
                    status="active"
                  />
                  <div className="muted tiny">{progress.message}</div>
                </div>
              ))}
          </Space>
        </Card>
      ) : null}

      {!data ? (
        <div style={{ padding: 60, textAlign: "center" }}>
          <Progress type="circle" percent={70} status="active" />
        </div>
      ) : data.totals.submissions === 0 && data.accounts.length === 0 ? (
        <EmptyState
          title="还没有训练数据"
          description="先同步 OJ，或写入一份演示数据看看完整效果。演示数据只包含虚构账号与题目。"
          action={
            <Space>
              <Button
                type="primary"
                icon={<CloudSyncOutlined />}
                onClick={() => startSync("codeforces")}
              >
                同步 Codeforces
              </Button>
              <Button onClick={seed}>写入演示数据</Button>
            </Space>
          }
        />
      ) : (
        <>
          <div className="stat-grid mb-14">
            <Card className="panel" size="small">
              <Statistic
                title="最高 Rating"
                value={maxRating}
                prefix={<TrophyOutlined />}
                styles={{ content: { color: "#67e8f9" } }}
              />
            </Card>
            <Card className="panel" size="small">
              <Statistic
                title="累计通过"
                value={data.totals.solved}
                suffix="题"
                prefix={<RiseOutlined />}
              />
            </Card>
            <Card className="panel" size="small">
              <Statistic
                title="通过率"
                value={(data.totals.acceptanceRate * 100).toFixed(1)}
                suffix="%"
                styles={{
                  content: {
                    color:
                      data.totals.acceptanceRate > 0.5 ? "#34d399" : "#fbbf24",
                  },
                }}
              />
            </Card>
            <Card className="panel" size="small">
              <Statistic
                title="连续训练"
                value={data.totals.streakDays}
                suffix="天"
                prefix={<FireOutlined />}
                styles={{ content: { color: "#fbbf24" } }}
              />
            </Card>
          </div>

          <div
            className="card-grid mb-14"
            style={{ gridTemplateColumns: "1.4fr 1fr" }}
          >
            <Card
              className="panel"
              size="small"
              title="Rating 趋势"
              extra={<span className="muted tiny">来自各平台同步</span>}
            >
              {data.ratingPoints.length ? (
                <EChart option={ratingOption} height={286} />
              ) : (
                <Empty description="暂无 rating 历史" />
              )}
            </Card>
            <Card
              className="panel"
              size="small"
              title="标签能力"
              extra={<span className="muted tiny">Elo 估计，越低越薄弱</span>}
            >
              {data.tagStats.length ? (
                <EChart option={tagOption} height={286} />
              ) : (
                <Empty description="提交几道带标签的题后自动生成" />
              )}
            </Card>
          </div>

          <div
            className="card-grid mb-14"
            style={{ gridTemplateColumns: "1.5fr 1fr" }}
          >
            <Card
              className="panel"
              size="small"
              title="最近提交"
              extra={<a onClick={() => navigate("/reports")}>生成报告</a>}
            >
              <Table<SubmissionRow>
                rowKey="id"
                size="small"
                pagination={false}
                columns={columns}
                dataSource={data.recentSubmissions}
                locale={{ emptyText: "暂无提交" }}
              />
            </Card>
            <Card
              className="panel"
              size="small"
              title="代码分析"
              extra={<span className="muted tiny">每次提交后自动更新</span>}
            >
              {data.analyses.length ? (
                <Space orientation="vertical" style={{ width: "100%" }} size={10}>
                  {data.analyses.map((analysis) => (
                    <div
                      key={analysis.submissionId}
                      style={{
                        borderBottom: "1px solid #1b2634",
                        paddingBottom: 8,
                      }}
                    >
                      <div className="flex between center gap-8">
                        <strong className="gz">{analysis.problemTitle}</strong>
                        <Tag color={verdictColor(analysis.verdict)}>
                          {verdictLabel(analysis.verdict)}
                        </Tag>
                      </div>
                      <div className="muted small" style={{ marginTop: 4 }}>
                        {analysis.summary}
                      </div>
                      <div className="muted tiny" style={{ marginTop: 2 }}>
                        {analysis.category} · {formatDate(analysis.createdAt)}
                      </div>
                    </div>
                  ))}
                </Space>
              ) : (
                <Empty description="提交后会自动生成分析" />
              )}
            </Card>
          </div>

          <Card
            className="panel"
            size="small"
            title="当前训练计划"
            extra={
              plan ? (
                <Space>
                  <span className="muted tiny">
                    {planDone}/{plan.items.length} 完成
                  </span>
                  <Button size="small" onClick={() => navigate("/training")}>
                    查看
                  </Button>
                </Space>
              ) : (
                <Button size="small" onClick={() => navigate("/training")}>
                  去生成
                </Button>
              )
            }
          >
            {plan ? (
              <>
                <Progress
                  percent={
                    plan.items.length
                      ? Math.round((planDone / plan.items.length) * 100)
                      : 0
                  }
                  size="small"
                />
                <div className="muted small mt-8">{plan.rationale}</div>
                <div className="flex wrap gap-8 mt-12">
                  {plan.items.map((item) => (
                    <Tooltip key={item.id} title={item.reason}>
                      <Tag
                        color={item.status === "done" ? "green" : "default"}
                        style={{ cursor: "pointer" }}
                        onClick={() => navigate(`/problem/${item.problemId}`)}
                      >
                        {item.title}
                        {item.rating ? ` · ${item.rating}` : ""}
                      </Tag>
                    </Tooltip>
                  ))}
                </div>
              </>
            ) : (
              <Empty description="还没有 active 计划" />
            )}
          </Card>
        </>
      )}
    </div>
  );
}
