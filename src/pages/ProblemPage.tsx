import {
  Alert,
  App,
  Button,
  Card,
  Checkbox,
  Collapse,
  Empty,
  Input,
  InputNumber,
  Modal,
  Select,
  Space,
  Spin,
  Tabs,
  Tag,
  Tooltip,
} from "antd";
import {
  ArrowLeftOutlined,
  CloudUploadOutlined,
  CodeOutlined,
  ExperimentOutlined,
  LinkOutlined,
  PlayCircleOutlined,
  ReloadOutlined,
  SendOutlined,
} from "@ant-design/icons";
import CodeMirror from "@uiw/react-codemirror";
import { cpp } from "@codemirror/lang-cpp";
import { autocompletion } from "@codemirror/autocomplete";
import { oneDark } from "@codemirror/theme-one-dark";
import { useEffect, useMemo, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useNavigate, useParams } from "react-router-dom";
import { api, errText } from "../lib/api";
import { cppCompletion } from "../lib/cppCompletion";
import {
  difficultyLabel,
  failureCategoryLabel,
  formatDate,
  platformName,
  verdictLabel,
} from "../lib/format";
import { MarkdownView } from "../lib/markdown";
import { useAppStore } from "../store";
import { EmptyState, PlatformTag, VerdictTag } from "../components/ui";
import type { Analysis, JudgeResult, Submission } from "../types";

const DEFAULT_CODE = `#include <bits/stdc++.h>
using namespace std;

int main() {
    ios::sync_with_stdio(false);
    cin.tie(nullptr);

    // TODO

    return 0;
}
`;

function AnalysisView({ analysis }: { analysis: Analysis }) {
  return (
    <div>
      <div className="flex between center wrap gap-8 mb-8">
        <Space>
          <Tag color="cyan">AI 分析</Tag>
          <Tag color={analysis.failureCategory === "none" ? "green" : "orange"}>
            {failureCategoryLabel(analysis.failureCategory)}
          </Tag>
        </Space>
        <span className="muted tiny">
          置信度 {(analysis.confidence * 100).toFixed(0)}% · {formatDate(analysis.createdAt)}
        </span>
      </div>
      <p style={{ margin: "4px 0 12px" }}>{analysis.summary}</p>
      {analysis.rootCause ? (
        <div className="sample-block">
          <div className="sample-label">根因</div>
          <div className="muted">{analysis.rootCause}</div>
        </div>
      ) : null}
      {analysis.bugPatterns.length ? (
        <div className="sample-block">
          <div className="sample-label">错误模式</div>
          <div className="flex wrap gap-8">
            {analysis.bugPatterns.map((pattern) => (
              <Tag key={pattern} color="volcano">
                {pattern}
              </Tag>
            ))}
          </div>
        </div>
      ) : null}
      {analysis.evidence.length ? (
        <div className="sample-block">
          <div className="sample-label">证据</div>
          <ul className="muted" style={{ margin: 0, paddingLeft: 18 }}>
            {analysis.evidence.map((line) => (
              <li key={line}>{line}</li>
            ))}
          </ul>
        </div>
      ) : null}
      <div className="sample-block">
        <div className="sample-label">复杂度</div>
        <div className="muted mono tiny">
          实际 {String(analysis.complexity.time ?? "?")} / 空间{" "}
          {String(analysis.complexity.space ?? "?")} · 期望{" "}
          {String(analysis.complexity.expected ?? "?")} ·{" "}
          {String(analysis.complexity.verdict ?? "unknown")}
        </div>
      </div>
      {analysis.fixSteps.length ? (
        <div className="sample-block">
          <div className="sample-label">最小修复步骤</div>
          <ol className="muted" style={{ margin: 0, paddingLeft: 18 }}>
            {analysis.fixSteps.map((step) => (
              <li key={step}>{step}</li>
            ))}
          </ol>
        </div>
      ) : null}
      {analysis.learningPoints.length ? (
        <div className="sample-block">
          <div className="sample-label">学习点</div>
          <ul className="muted" style={{ margin: 0, paddingLeft: 18 }}>
            {analysis.learningPoints.map((point) => (
              <li key={point}>{point}</li>
            ))}
          </ul>
        </div>
      ) : null}
      {analysis.nextPracticeTags.length ? (
        <div className="flex wrap gap-8 mt-8">
          <span className="muted tiny">下次练：</span>
          {analysis.nextPracticeTags.map((tag) => (
            <Tag key={tag}>{tag}</Tag>
          ))}
        </div>
      ) : null}
    </div>
  );
}

export default function ProblemPage() {
  const { problemId = "" } = useParams();
  const { message } = App.useApp();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const settings = useAppStore((s) => s.settings);
  const requestAutomation = useAppStore((s) => s.requestAutomation);
  const setAutomationOpen = useAppStore((s) => s.setAutomationOpen);
  const setAutomationContext = useAppStore((s) => s.setAutomationContext);
  const submitResult = useAppStore((s) => s.submitResult);
  const setSubmitResult = useAppStore((s) => s.setSubmitResult);

  const [code, setCode] = useState(DEFAULT_CODE);
  const [draftLoaded, setDraftLoaded] = useState(false);
  const [compiler, setCompiler] = useState<string | undefined>(undefined);
  const [standard, setStandard] = useState(settings?.compilerStandard ?? "gnu++17");
  const [judge, setJudge] = useState<JudgeResult | null>(null);
  const [running, setRunning] = useState(false);
  const [submitOpen, setSubmitOpen] = useState(false);
  const [autoSubmit, setAutoSubmit] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [manualOpen, setManualOpen] = useState(false);
  const [manualVerdict, setManualVerdict] = useState("AC");
  const [manualTime, setManualTime] = useState<number | null>(null);
  const [manualMemory, setManualMemory] = useState<number | null>(null);
  const [lastSubmission, setLastSubmission] = useState<Submission | null>(null);
  const [tab, setTab] = useState("tests");
  const saveTimer = useRef<number | null>(null);

  const problem = useQuery({
    queryKey: ["problem", problemId],
    queryFn: () => api.getProblem(problemId),
    enabled: !!problemId,
  });
  const draft = useQuery({
    queryKey: ["draft", problemId],
    queryFn: () => api.getDraft(problemId),
    enabled: !!problemId,
  });
  const toolchains = useQuery({ queryKey: ["toolchains"], queryFn: api.detectToolchains });
  const submissions = useQuery({
    queryKey: ["submissions", 100],
    queryFn: () => api.listSubmissions(100),
  });
  const problemSubmissions = useMemo(
    () => (submissions.data ?? []).filter((item) => item.problemId === problemId),
    [submissions.data, problemId],
  );
  const analysis = useQuery({
    queryKey: ["analysis", lastSubmission?.id],
    queryFn: () => api.getAnalysis(lastSubmission!.id),
    enabled: !!lastSubmission,
  });

  useEffect(() => {
    if (draftLoaded || !draft.isFetched) return;
    if (draft.data?.code) setCode(draft.data.code);
    setDraftLoaded(true);
  }, [draft.data, draft.isFetched, draftLoaded]);

  useEffect(() => {
    if (settings?.compilerPath) setCompiler(settings.compilerPath);
    if (settings?.compilerStandard) setStandard(settings.compilerStandard);
  }, [settings?.compilerPath, settings?.compilerStandard]);

  useEffect(() => {
    if (compiler || !toolchains.data?.length) return;
    const preferred =
      toolchains.data.find((tool) => tool.supportsBits) ?? toolchains.data[0];
    setCompiler(preferred.path);
  }, [compiler, toolchains.data]);

  useEffect(() => {
    if (!draftLoaded || !problemId) return;
    if (saveTimer.current) window.clearTimeout(saveTimer.current);
    saveTimer.current = window.setTimeout(() => {
      api.saveDraft(problemId, code).catch(() => {});
    }, 900);
    return () => {
      if (saveTimer.current) window.clearTimeout(saveTimer.current);
    };
  }, [code, draftLoaded, problemId]);

  useEffect(() => {
    if (problemSubmissions.length && !lastSubmission) {
      const row = problemSubmissions[0];
      setLastSubmission({
        ...row,
        memoryKb: null,
        needsHuman: false,
        analysis: null,
      });
    }
  }, [problemSubmissions, lastSubmission]);

  useEffect(() => {
    if (!lastSubmission) return;
    if (lastSubmission.analysisStatus !== "pending" && lastSubmission.analysisStatus !== "running") {
      return;
    }
    const timer = window.setInterval(async () => {
      try {
        const fresh = await api.getSubmission(lastSubmission.id);
        setLastSubmission(fresh);
        queryClient.invalidateQueries({ queryKey: ["analysis", fresh.id] });
      } catch {
        // ignore transient polling errors
      }
    }, 3_000);
    return () => window.clearInterval(timer);
  }, [lastSubmission, queryClient]);

  useEffect(() => {
    if (!submitResult?.submission) return;
    if (submitResult.submission.problemId !== problemId) return;
    setLastSubmission(submitResult.submission);
    setTab("analysis");
    setSubmitResult(null);
    queryClient.invalidateQueries({ queryKey: ["submissions", 100] });
    queryClient.invalidateQueries({ queryKey: ["dashboard"] });
    queryClient.invalidateQueries({ queryKey: ["plan"] });
  }, [submitResult, problemId, setSubmitResult, queryClient]);

  const run = async () => {
    if (!problem.data) return;
    if (!problem.data.samples.length) {
      message.warning("这道题还没有解析到样例，可以先在浏览器中查看题面。");
      return;
    }
    setRunning(true);
    setTab("tests");
    try {
      const result = await api.runSamples({
        problemId,
        code,
        compilerPath: compiler,
        standard,
        saveDraft: true,
      });
      setJudge(result);
      if (!result.compileOk) message.error("编译失败，请查看编译输出");
      else if (result.ok) message.success("全部样例通过");
      else message.warning("样例未全部通过");
    } catch (error) {
      message.error(errText(error));
    } finally {
      setRunning(false);
    }
  };

  const submit = async () => {
    setSubmitting(true);
    setAutomationOpen(true);
    setAutomationContext({
      problemId,
      code,
      language: "cpp",
      since: Math.floor(Date.now() / 1000) - 5,
    });
    try {
      const result = await api.submitSolution({
        problemId,
        code,
        language: "cpp",
        autoSubmit,
      });
      if (result.submission) {
        setLastSubmission(result.submission);
        setTab("analysis");
        queryClient.invalidateQueries({ queryKey: ["submissions", 100] });
        queryClient.invalidateQueries({ queryKey: ["dashboard"] });
        queryClient.invalidateQueries({ queryKey: ["plan"] });
        message.success(result.message);
      } else {
        message.warning(result.message);
      }
      setSubmitOpen(false);
    } catch (error) {
      message.error(errText(error));
    } finally {
      setSubmitting(false);
    }
  };

  const recordManual = async () => {
    try {
      const submission = await api.recordManualSubmission({
        problemId,
        code,
        verdict: manualVerdict,
        language: "cpp",
        timeMs: manualTime ?? undefined,
        memoryKb: manualMemory ?? undefined,
      });
      setLastSubmission(submission);
      setManualOpen(false);
      setTab("analysis");
      queryClient.invalidateQueries({ queryKey: ["submissions", 100] });
      queryClient.invalidateQueries({ queryKey: ["dashboard"] });
      message.success("已记录提交，正在分析");
    } catch (error) {
      message.error(errText(error));
    }
  };

  if (problem.isLoading) {
    return (
      <div style={{ height: "100%", display: "grid", placeItems: "center" }}>
        <Spin />
      </div>
    );
  }
  if (!problem.data) {
    return (
      <div className="page">
        <EmptyState
          title="题目加载失败"
          description={problem.error ? errText(problem.error) : "请返回训练页重新选择题目。"}
          action={<Button onClick={() => navigate("/training")}>返回训练</Button>}
        />
      </div>
    );
  }

  const current = problem.data;
  const compileErrors = judge && !judge.compileOk;
  const testContent = (
    <div>
      {compileErrors ? (
        <Alert
          type="error"
          showIcon
          title="编译失败"
          description={
            <pre className="sample-pre" style={{ maxHeight: 220 }}>
              {judge?.compileOutput || "（无输出）"}
            </pre>
          }
          style={{ marginBottom: 12 }}
        />
      ) : null}
      {judge?.compileOk ? (
        <div className="flex between center mb-8">
          <span className="muted tiny">
            编译通过 · 总耗时 {judge.totalMs} ms · {judge.compiler ?? ""}
          </span>
          <Tag color={judge.ok ? "green" : "orange"}>
            {judge.ok ? "全部样例通过" : "存在失败样例"}
          </Tag>
        </div>
      ) : null}
      {current.samples.length ? (
        current.samples.map((sample, index) => {
          const result = judge?.samples.find((item) => item.index === index + 1);
          return (
            <div className="sample-block" key={index}>
              <div className="flex between center">
                <span className="sample-label">样例 {index + 1}</span>
                {result ? (
                  <Tag color={result.status === "OK" ? "green" : "red"}>
                    {result.status === "OK" ? "通过" : result.status} · {result.timeMs} ms
                  </Tag>
                ) : null}
              </div>
              <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 8 }}>
                <div>
                  <div className="muted tiny">输入</div>
                  <pre className="sample-pre">{sample.input || "（空）"}</pre>
                </div>
                <div>
                  <div className="muted tiny">期望输出</div>
                  <pre className="sample-pre">{sample.output || "（空）"}</pre>
                </div>
              </div>
              {result && result.status !== "OK" ? (
                <Collapse
                  ghost
                  size="small"
                  items={[
                    {
                      key: "detail",
                      label: <span className="muted tiny">查看实际输出 / 差异</span>,
                      children: (
                        <div>
                          {result.diff ? (
                            <pre className="sample-pre" style={{ maxHeight: 160 }}>
                              {result.diff}
                            </pre>
                          ) : null}
                          <div className="muted tiny">实际输出</div>
                          <pre className="sample-pre" style={{ maxHeight: 160 }}>
                            {result.stdout || result.stderr || "（空）"}
                          </pre>
                        </div>
                      ),
                    },
                  ]}
                />
              ) : null}
            </div>
          );
        })
      ) : (
        <Alert
          type="info"
          showIcon
          title="还没有样例"
          description="题面可能尚未解析完成，或平台页面结构发生了变化。可以重新获取题面，或直接在右侧自动化面板查看题目。"
          action={
            <Button
              size="small"
              onClick={() => {
                problem.refetch();
                requestAutomation(current.platform, current.url);
              }}
            >
              重新获取
            </Button>
          }
        />
      )}
    </div>
  );

  const historyContent = (
    <div>
      {problemSubmissions.length ? (
        <Space orientation="vertical" style={{ width: "100%" }}>
          {problemSubmissions.map((item) => (
            <Card
              key={item.id}
              size="small"
              style={{ background: "#0f151d", borderColor: "#1b2634" }}
              onClick={() => {
                setLastSubmission({
                  ...item,
                  memoryKb: null,
                  needsHuman: false,
                  analysis: null,
                });
                setTab("analysis");
              }}
              hoverable
            >
              <div className="flex between center gap-8">
                <Space>
                  <VerdictTag verdict={item.verdict} />
                  <span className="muted small">{item.language ?? "C++"}</span>
                  <span className="muted tiny">{item.timeMs ? `${item.timeMs} ms` : ""}</span>
                </Space>
                <span className="muted tiny">{formatDate(item.submittedAt)}</span>
              </div>
            </Card>
          ))}
        </Space>
      ) : (
        <Empty description="还没有提交记录" />
      )}
      <div className="mt-12">
        <Button onClick={() => setManualOpen(true)}>手动录入一次提交结果</Button>
      </div>
    </div>
  );

  const analysisContent = analysis.isLoading ? (
    <Spin />
  ) : analysis.data ? (
    <AnalysisView analysis={analysis.data} />
  ) : lastSubmission ? (
    <Alert
      type="info"
      showIcon
      title={`提交结果：${verdictLabel(lastSubmission.verdict)}`}
      description={
        lastSubmission.analysisStatus === "pending" ||
        lastSubmission.analysisStatus === "running"
          ? "分析进行中，通常需要 10-40 秒。"
          : "这次提交还没有 AI 分析。可能是未配置分析模型，或代码没有保存。"
      }
      action={
        <Button
          size="small"
          onClick={async () => {
            try {
              await api.reanalyzeSubmission(lastSubmission.id);
              queryClient.invalidateQueries({
                queryKey: ["analysis", lastSubmission.id],
              });
              message.success("已重新开始分析");
            } catch (error) {
              message.error(errText(error));
            }
          }}
        >
          重新分析
        </Button>
      }
    />
  ) : (
    <Empty description="提交后会自动生成代码分析" />
  );

  return (
    <div className="problem-layout">
      <div className="problem-statement">
        <Space className="mb-8">
          <Button
            type="text"
            size="small"
            icon={<ArrowLeftOutlined />}
            onClick={() => navigate("/training")}
          >
            返回
          </Button>
          <PlatformTag platform={current.platform} />
          <Tag>{current.rating ?? "未定级"} · {difficultyLabel(current.rating)}</Tag>
          {current.solved ? <Tag color="green">已通过</Tag> : null}
          {current.attempted && !current.solved ? <Tag color="orange">尝试过</Tag> : null}
        </Space>
        <h1 style={{ fontSize: 24, lineHeight: 1.3, margin: "4px 0 10px" }}>
          {current.title}
        </h1>
        <Space wrap className="mb-14">
          {current.tags.map((tag) => (
            <Tag key={tag}>{tag}</Tag>
          ))}
          <a href={current.url} target="_blank" rel="noreferrer">
            <LinkOutlined /> 在浏览器中打开
          </a>
        </Space>
        <div className="muted tiny mb-14">
          时限 {current.timeLimitMs ?? "?"} ms · 内存 {current.memoryLimitMb ?? "?"} MB
        </div>
        {current.statementMd ? (
          <MarkdownView content={current.statementMd} />
        ) : (
          <Alert
            type="warning"
            showIcon
            title="题面尚未加载"
            description="点击下方按钮在自动化面板打开原题页，或重新获取题面。"
            action={
              <Space>
                <Button size="small" onClick={() => problem.refetch()}>
                  <ReloadOutlined /> 重新获取
                </Button>
                <Button
                  size="small"
                  onClick={() => requestAutomation(current.platform, current.url)}
                >
                  打开原题页
                </Button>
              </Space>
            }
          />
        )}
        {current.samples.length ? (
          <div className="mt-18">
            <h3 style={{ fontSize: 15 }}>样例</h3>
            {current.samples.map((sample, index) => (
              <div className="sample-block" key={index}>
                <div className="sample-label">样例 {index + 1}</div>
                <div className="sample-pre">{sample.input || "（空）"}</div>
                <div className="sample-label mt-8">输出</div>
                <div className="sample-pre">{sample.output || "（空）"}</div>
              </div>
            ))}
          </div>
        ) : null}
      </div>

      <div className="problem-workspace">
        <div className="editor-toolbar">
          <Space size={8} wrap>
            <CodeOutlined style={{ color: "#67e8f9" }} />
            <Select
              size="small"
              value={compiler}
              onChange={setCompiler}
              style={{ width: 190 }}
              placeholder="编译器"
              options={(toolchains.data ?? []).map((tool) => ({
                value: tool.path,
                label: `${tool.path}${tool.supportsBits ? " · GNU" : ""}`,
              }))}
            />
            <Select
              size="small"
              value={standard}
              onChange={setStandard}
              style={{ width: 120 }}
              options={[
                { value: "gnu++17", label: "GNU++17" },
                { value: "gnu++20", label: "GNU++20" },
                { value: "gnu++23", label: "GNU++23" },
              ]}
            />
          </Space>
          <span className="grow" />
          <Space wrap>
            <Button
              size="small"
              icon={<PlayCircleOutlined />}
              loading={running}
              onClick={run}
            >
              运行样例
            </Button>
            <Button
              size="small"
              icon={<CloudUploadOutlined />}
              onClick={() => {
                setAutoSubmit(
                  settings?.autoSubmit === true && settings?.requireSubmitConfirm !== true,
                );
                setSubmitOpen(true);
              }}
            >
              提交到 OJ
            </Button>
            <Button
              size="small"
              icon={<LinkOutlined />}
              onClick={() => requestAutomation(current.platform, current.url)}
            >
              打开页面
            </Button>
          </Space>
        </div>
        <div className="editor-area">
          <CodeMirror
            value={code}
            height="100%"
            theme={oneDark}
            extensions={[cpp(), autocompletion({ override: [cppCompletion] })]}
            onChange={(value) => setCode(value)}
            basicSetup={{
              lineNumbers: true,
              foldGutter: true,
              highlightActiveLine: true,
              autocompletion: false,
            }}
          />
        </div>
        <div className="test-panel">
          <Tabs
            size="small"
            activeKey={tab}
            onChange={setTab}
            style={{ height: "100%" }}
            tabBarStyle={{ margin: 0, padding: "0 12px" }}
            items={[
              {
                key: "tests",
                label: (
                  <span>
                    <ExperimentOutlined /> 样例测试
                  </span>
                ),
                children: <div className="test-panel-body">{testContent}</div>,
              },
              {
                key: "history",
                label: (
                  <span>
                    <CodeOutlined /> 提交记录
                  </span>
                ),
                children: <div className="test-panel-body">{historyContent}</div>,
              },
              {
                key: "analysis",
                label: (
                  <span>
                    <SendOutlined /> 代码分析
                  </span>
                ),
                children: <div className="test-panel-body">{analysisContent}</div>,
              },
            ]}
          />
        </div>
      </div>

      <Modal
        open={submitOpen}
        title="提交到 OJ"
        onCancel={() => setSubmitOpen(false)}
        onOk={submit}
        confirmLoading={submitting}
        okText={autoSubmit ? "自动提交" : "填表并等待确认"}
        cancelText="取消"
        width={600}
      >
        <Space orientation="vertical" style={{ width: "100%" }} size={12}>
          <div>
            <Tag color="blue">{platformName(current.platform)}</Tag>
            <strong>{current.title}</strong>
          </div>
          <div className="muted small">
            代码 {code.length} 字符 · 语言 C++ · 目标 {standard}
          </div>
          <Alert
            type="warning"
            showIcon
            title="提交会操作你的真实 OJ 账号"
            description="若平台触发人机验证，应用会暂停并把页面留给你操作。相同的代码在 10 分钟内不会被重复提交。"
          />
          <Checkbox
            checked={autoSubmit}
            onChange={(event) => setAutoSubmit(event.target.checked)}
          >
            自动点击提交（不勾选时只填表，由你在浏览器中确认）
          </Checkbox>
          <div className="muted tiny">
            第一次使用建议不勾选：先确认语言与代码，再手动点提交，然后在自动化面板点“我已提交，读取结果”。
          </div>
        </Space>
      </Modal>

      <Modal
        open={manualOpen}
        title="手动录入提交结果"
        onCancel={() => setManualOpen(false)}
        onOk={recordManual}
        okText="保存并分析"
        cancelText="取消"
      >
        <Space orientation="vertical" style={{ width: "100%" }}>
          <div>
            <div className="muted tiny mb-8">判定</div>
            <Select
              value={manualVerdict}
              onChange={setManualVerdict}
              style={{ width: "100%" }}
              options={["AC", "WA", "TLE", "MLE", "RE", "CE", "PARTIAL"].map((value) => ({
                value,
                label: `${value} · ${verdictLabel(value)}`,
              }))}
            />
          </div>
          <div className="flex gap-8">
            <div className="grow">
              <div className="muted tiny mb-8">用时（ms，可选）</div>
              <InputNumber
                value={manualTime}
                onChange={setManualTime}
                style={{ width: "100%" }}
                min={0}
              />
            </div>
            <div className="grow">
              <div className="muted tiny mb-8">内存（KB，可选）</div>
              <InputNumber
                value={manualMemory}
                onChange={setManualMemory}
                style={{ width: "100%" }}
                min={0}
              />
            </div>
          </div>
          <div className="muted tiny">
            用于平台提交被风控阻断时手工补录；会照常触发代码分析与训练计划刷新。
          </div>
        </Space>
      </Modal>
    </div>
  );
}
