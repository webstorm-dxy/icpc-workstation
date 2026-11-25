import {
  App,
  Button,
  Card,
  Empty,
  InputNumber,
  Modal,
  Progress,
  Space,
  Tag,
  Tooltip,
} from "antd";
import { ReloadOutlined, RocketOutlined, SyncOutlined } from "@ant-design/icons";
import { useEffect, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "react-router-dom";
import { api, errText } from "../lib/api";
import { MarkdownView } from "../lib/markdown";
import { useAppStore } from "../store";
import { difficultyLabel, planKindLabel, platformName } from "../lib/format";
import { EmptyState, PageHeader, PlatformTag } from "../components/ui";
import type { PlanItem } from "../types";

const PLAN_PLATFORMS = ["codeforces", "atcoder", "luogu"] as const;
const MIN_PLAN_SIZE = 4;
const MAX_PLAN_SIZE = 12;

export default function TrainingPage() {
  const { message } = App.useApp();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [generating, setGenerating] = useState(false);
  const [quota, setQuota] = useState<Record<string, number> | null>(null);
  const openPlanAgent = useAppStore((s) => s.openPlanAgent);
  const applyPlanAgent = useAppStore((s) => s.applyPlanAgent);
  const plan = useQuery({ queryKey: ["plan"], queryFn: api.getActivePlan });

  // Start from the current plan's platform mix so "generate again" keeps it.
  useEffect(() => {
    if (quota !== null) return;
    const items = plan.data?.items;
    if (!items) return;
    const counts: Record<string, number> = {};
    for (const item of items) {
      counts[item.platform] = (counts[item.platform] ?? 0) + 1;
    }
    setQuota(counts);
  }, [plan.data, quota]);

  const quotaByPlatform = quota ?? {};
  const quotaTotal = PLAN_PLATFORMS.reduce(
    (sum, platform) => sum + (quotaByPlatform[platform] ?? 0),
    0,
  );
  const setPlatformCount = (platform: string, value: number | null) => {
    setQuota({ ...quotaByPlatform, [platform]: Math.max(0, Math.min(MAX_PLAN_SIZE, value ?? 0)) });
  };

  const generate = async () => {
    if (quotaTotal > 0 && quotaTotal < MIN_PLAN_SIZE) {
      message.warning(`至少选择 ${MIN_PLAN_SIZE} 道题，或者把各平台都设为 0 交给 Agent 自由挑选`);
      return;
    }
    if (quotaTotal > MAX_PLAN_SIZE) {
      message.warning(`一次最多 ${MAX_PLAN_SIZE} 道题`);
      return;
    }
    const platforms = PLAN_PLATFORMS.map((platform) => ({
      platform,
      count: quotaByPlatform[platform] ?? 0,
    })).filter((item) => item.count > 0);
    openPlanAgent();
    setGenerating(true);
    message.loading({ content: "Agent 正在分析提交、代码与能力画像…", key: "plan" });
    try {
      await api.generatePlan({
        reason: "训练页手动生成",
        count: platforms.length ? undefined : 8,
        platforms,
      });
      message.success({ content: "训练计划已更新", key: "plan" });
      await queryClient.invalidateQueries({ queryKey: ["plan"] });
      await queryClient.invalidateQueries({ queryKey: ["dashboard"] });
    } catch (error) {
      const detail = errText(error);
      message.error({ content: detail, key: "plan" });
      applyPlanAgent({ phase: "error", trigger: "manual", message: detail });
    } finally {
      setGenerating(false);
    }
  };

  const mixLabel = PLAN_PLATFORMS.filter((platform) => (quotaByPlatform[platform] ?? 0) > 0)
    .map((platform) => `${platformName(platform)} ${quotaByPlatform[platform]} 道`)
    .join("、");

  const refresh = () => {
    Modal.confirm({
      title: "重新生成训练计划？",
      content:
        (mixLabel ? `将按 ${mixLabel} 的配额选题。` : "本次不限来源平台，由 Agent 自由挑选。") +
        "Agent 会先分析最近提交、可用的提交代码与既有分析，形成诊断后再挑选新题目，并替换当前 active 计划。历史计划会保留。",
      okText: "重新生成",
      cancelText: "取消",
      onOk: generate,
    });
  };

  const setStatus = async (item: PlanItem, status: string) => {
    try {
      await api.setPlanItemStatus(item.id, status);
      await queryClient.invalidateQueries({ queryKey: ["plan"] });
      await queryClient.invalidateQueries({ queryKey: ["dashboard"] });
    } catch (error) {
      message.error(errText(error));
    }
  };

  const data = plan.data;
  const done = data?.items.filter((item) => item.status === "done").length ?? 0;
  const percent = data?.items.length ? Math.round((done / data.items.length) * 100) : 0;

  return (
    <div className="page">
      <PageHeader
        title="训练"
        subtitle="由 Agent 根据你的 rating、标签能力与最近提交生成；每次提交后会自动刷新。"
        extra={
          <Space>
            <Button
              icon={<ReloadOutlined />}
              onClick={() => plan.refetch()}
              loading={plan.isFetching}
            >
              刷新
            </Button>
            <Button type="primary" icon={<RocketOutlined />} loading={generating} onClick={refresh}>
              生成新计划
            </Button>
          </Space>
        }
      />

      <Card
        className="panel mb-14"
        size="small"
        title="题目来源"
        extra={
          <span className="muted tiny">
            {quotaTotal > 0 ? `共 ${quotaTotal} 题` : "不限平台"}
          </span>
        }
      >
        <div className="flex between center wrap gap-12">
          <Space wrap size={16}>
            {PLAN_PLATFORMS.map((platform) => (
              <Space key={platform} size={6}>
                <PlatformTag platform={platform} />
                <InputNumber
                  size="small"
                  min={0}
                  max={MAX_PLAN_SIZE}
                  value={quotaByPlatform[platform] ?? 0}
                  onChange={(value) => setPlatformCount(platform, value)}
                  style={{ width: 70 }}
                />
              </Space>
            ))}
          </Space>
          <span className="muted tiny">
            按平台分配题数，生成时各平台严格按配额出题；全部为 0 时由 Agent 自由挑选（默认 8 题）。
          </span>
        </div>
      </Card>

      {plan.isLoading ? (
        <div style={{ padding: 60, textAlign: "center" }}>
          <SyncOutlined spin style={{ fontSize: 28, color: "#22d3ee" }} />
        </div>
      ) : !data ? (
        <EmptyState
          title="还没有训练计划"
          description="先同步至少一个 OJ 的题池，然后让 Agent 生成一份 6-10 题的训练计划。也可以写入演示数据先看看界面。"
          action={
            <Space>
              <Button type="primary" icon={<RocketOutlined />} loading={generating} onClick={generate}>
                生成训练计划
              </Button>
              <Button onClick={() => navigate("/dashboard")}>去同步数据</Button>
            </Space>
          }
        />
      ) : (
        <>
          <Card className="panel mb-14" size="small">
            <div className="flex between center wrap gap-12">
              <div className="grow">
                <div className="flex center gap-8 wrap">
                  <strong>本次计划</strong>
                  <Tag color="cyan">{data.items.length} 题</Tag>
                  <Tag>{done} 已完成</Tag>
                  <span className="muted tiny">生成于 {data.createdAt.slice(0, 10)}</span>
                </div>
                <div className="plan-rationale">
                  <MarkdownView content={data.rationale} />
                </div>
              </div>
              <div style={{ width: 220 }}>
                <Progress percent={percent} size="small" />
              </div>
            </div>
          </Card>

          <div className="plan-grid">
            {data.items.map((item) => (
              <Card
                key={item.id}
                className="panel"
                size="small"
                title={
                  <div className="flex between center gap-8">
                    <span
                      style={{ cursor: "pointer", overflow: "hidden", textOverflow: "ellipsis" }}
                      onClick={() => navigate(`/problem/${item.problemId}`)}
                    >
                      {item.title}
                    </span>
                    <PlatformTag platform={item.platform} />
                  </div>
                }
                extra={
                  <Tag color={item.kind === "stretch" ? "purple" : item.kind === "review" ? "orange" : "blue"}>
                    {planKindLabel(item.kind)}
                  </Tag>
                }
                actions={[
                  <Button
                    key="start"
                    type="link"
                    onClick={() => navigate(`/problem/${item.problemId}`)}
                  >
                    开始练习
                  </Button>,
                  <Tooltip key="done" title="标记完成后会更新计划进度">
                    <Button
                      type="link"
                      disabled={item.status === "done"}
                      onClick={() => setStatus(item, "done")}
                    >
                      {item.status === "done" ? "已完成" : "标记完成"}
                    </Button>
                  </Tooltip>,
                  <Button
                    key="skip"
                    type="link"
                    style={{ color: "#8b9bb0" }}
                    onClick={() => setStatus(item, "skipped")}
                  >
                    跳过
                  </Button>,
                ]}
              >
                <div className="flex wrap gap-8 mb-8">
                  {item.rating ? (
                    <Tag color="geekblue">
                      {item.rating} · {difficultyLabel(item.rating)}
                    </Tag>
                  ) : null}
                  {item.expectedMinutes ? (
                    <Tag>预计 {item.expectedMinutes} 分钟</Tag>
                  ) : null}
                  {item.status === "done" ? <Tag color="green">已完成</Tag> : null}
                  {item.status === "skipped" ? <Tag>已跳过</Tag> : null}
                </div>
                <div className="muted small" style={{ minHeight: 44 }}>
                  {item.reason}
                </div>
                <div className="flex wrap gap-8 mt-8">
                  {item.tags.slice(0, 4).map((tag) => (
                    <Tag key={tag} className="tiny">
                      {tag}
                    </Tag>
                  ))}
                </div>
              </Card>
            ))}
          </div>
          {!data.items.length ? <Empty description="计划里还没有题目" /> : null}
        </>
      )}
    </div>
  );
}
