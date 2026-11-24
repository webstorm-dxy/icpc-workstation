import {
  Alert,
  App,
  AutoComplete,
  Button,
  Card,
  Checkbox,
  Divider,
  Input,
  Progress,
  Space,
  Steps,
  Tag,
  Typography,
} from "antd";
import { CheckCircleOutlined, CloudSyncOutlined, LoginOutlined } from "@ant-design/icons";
import { useEffect, useMemo, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "react-router-dom";
import { api, errText } from "../lib/api";
import { platformName } from "../lib/format";
import { useAppStore } from "../store";

const PLATFORM_META: Record<string, { desc: string; color: string }> = {
  codeforces: {
    desc: "官方 API 读取 rating/提交；题面与提交走内置浏览器。",
    color: "blue",
  },
  atcoder: {
    desc: "kenkoooo 公开数据读取比赛/提交；提交走内置浏览器。",
    color: "purple",
  },
  luogu: {
    desc: "登录后读取个人数据；反爬较强，必要时人工接管。",
    color: "gold",
  },
};

const PROVIDERS = [
  {
    kind: "opencodego",
    label: "OpenCode Go",
    base: "https://opencode.ai/inference/go",
    model: "deepseek-v4.1-flash",
    desc: "默认推荐。订阅制，DeepSeek V4.1 Flash 走 Go 额度。",
    badge: "推荐",
  },
  {
    kind: "deepseek",
    label: "DeepSeek",
    base: "https://api.deepseek.com",
    model: "deepseek-flash",
    desc: "DeepSeek 官方 API，默认模型 deepseek-flash（V4.1 Flash）。",
    badge: "推荐",
  },
  {
    kind: "openai",
    label: "OpenAI",
    base: "https://api.openai.com/v1",
    model: "",
    desc: "使用 OpenAI API Key，模型从账号可用列表中选取。",
  },
  {
    kind: "anthropic",
    label: "Anthropic",
    base: "https://api.anthropic.com/v1",
    model: "",
    desc: "Claude API；使用 Anthropic Messages 协议。",
  },
  {
    kind: "custom",
    label: "自定义兼容端点",
    base: "",
    model: "",
    desc: "任意 OpenAI Chat Completions 兼容端点。",
  },
];

export default function OnboardingPage() {
  const { message } = App.useApp();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const onboarding = useAppStore((s) => s.onboarding);
  const setOnboarding = useAppStore((s) => s.setOnboarding);
  const requestAutomation = useAppStore((s) => s.requestAutomation);
  const sync = useAppStore((s) => s.sync);
  const [step, setStep] = useState(0);
  const [selected, setSelected] = useState<string[]>([]);
  const [handles, setHandles] = useState<Record<string, string>>({});
  const [kind, setKind] = useState("opencodego");
  const [base, setBase] = useState(PROVIDERS[0].base);
  const [apiKey, setApiKey] = useState("");
  const [model, setModel] = useState(PROVIDERS[0].model);
  const [models, setModels] = useState<string[]>([]);
  const [testing, setTesting] = useState(false);
  const [saving, setSaving] = useState(false);
  const [syncStarted, setSyncStarted] = useState<string[]>([]);

  const platformsQuery = useQuery({
    queryKey: ["platforms"],
    queryFn: api.listPlatforms,
  });
  const providersQuery = useQuery({
    queryKey: ["providers"],
    queryFn: api.listProviders,
  });

  useEffect(() => {
    if (!onboarding) return;
    if (onboarding.platforms.length) setSelected(onboarding.platforms);
    setStep(Math.min(Math.max(onboarding.step, 0), 4));
  }, [onboarding]);

  useEffect(() => {
    if (!platformsQuery.data) return;
    const next: Record<string, string> = {};
    for (const platform of platformsQuery.data) {
      next[platform.id] = platform.handle ?? "";
    }
    setHandles(next);
  }, [platformsQuery.data]);

  const existingProvider = useMemo(
    () => providersQuery.data?.find((p) => p.kind === kind),
    [providersQuery.data, kind],
  );

  const chooseProvider = (nextKind: string) => {
    const preset = PROVIDERS.find((p) => p.kind === nextKind);
    setKind(nextKind);
    setBase(preset?.base ?? "");
    setModel(preset?.model ?? "");
    setModels([]);
    setApiKey("");
    const existing = providersQuery.data?.find((p) => p.kind === nextKind);
    if (existing) {
      setBase(existing.baseUrl);
      const existingModel = String(existing.config?.default_model ?? "");
      if (existingModel) setModel(existingModel);
    }
  };

  const persistStep = async (next: number) => {
    setStep(next);
    try {
      await api.setOnboardingStep(next);
    } catch {
      // non-blocking
    }
  };

  const togglePlatform = (id: string, checked: boolean) => {
    setSelected((current) =>
      checked ? Array.from(new Set([...current, id])) : current.filter((p) => p !== id),
    );
  };

  const savePlatforms = async () => {
    if (selected.length === 0) {
      message.warning("至少选择一个 OJ 平台");
      return;
    }
    try {
      await api.setOnboardingPlatforms(selected);
      await persistStep(2);
    } catch (error) {
      message.error(errText(error));
    }
  };

  const saveHandle = async (platform: string) => {
    try {
      await api.savePlatformAccount(platform, handles[platform] ?? "");
      await platformsQuery.refetch();
      message.success(`${platformName(platform)} 账号已保存`);
    } catch (error) {
      message.error(errText(error));
    }
  };

  const verify = async (platform: string) => {
    try {
      const result = await api.verifyPlatformLogin(platform);
      if (result.loggedIn) message.success(result.message);
      else message.warning(result.message);
      await platformsQuery.refetch();
    } catch (error) {
      message.error(errText(error));
    }
  };

  const openLogin = (platform: string) => {
    const target = platformsQuery.data?.find((p) => p.id === platform);
    requestAutomation(platform, target?.loginUrl ?? "");
  };

  const testProvider = async () => {
    if (!base.trim()) {
      message.warning("请填写 Base URL");
      return;
    }
    if (!apiKey.trim() && !existingProvider?.hasKey) {
      message.warning("请填写 API Key");
      return;
    }
    setTesting(true);
    try {
      const result = await api.testProvider({
        id: existingProvider?.id,
        kind,
        name: PROVIDERS.find((p) => p.kind === kind)?.label ?? kind,
        baseUrl: base.trim(),
        apiKey: apiKey.trim() || undefined,
        config: { default_model: model.trim() },
      });
      if (result.ok) {
        setModels(result.models.map((m) => m.id));
        if (!model && result.models[0]) setModel(result.models[0].id);
        message.success(result.message);
      } else {
        message.error(result.message);
      }
    } catch (error) {
      message.error(errText(error));
    } finally {
      setTesting(false);
    }
  };

  const saveProvider = async () => {
    if (!base.trim()) {
      message.warning("请填写 Base URL");
      return;
    }
    if (!apiKey.trim() && !existingProvider?.hasKey) {
      message.warning('请填写 API Key，或先点“测试连接”');
      return;
    }
    setSaving(true);
    try {
      await api.upsertProvider({
        id: existingProvider?.id,
        kind,
        name: PROVIDERS.find((p) => p.kind === kind)?.label ?? kind,
        baseUrl: base.trim(),
        apiKey: apiKey.trim() || undefined,
        config: { default_model: model.trim() },
      });
      await queryClient.invalidateQueries({ queryKey: ["providers"] });
      await queryClient.invalidateQueries({ queryKey: ["role-models"] });
      await queryClient.invalidateQueries({ queryKey: ["bootstrap"] });
      await providersQuery.refetch();
      message.success("提供商已保存，四个 Agent 角色会自动使用该模型");
      await persistStep(4);
    } catch (error) {
      message.error(errText(error));
    } finally {
      setSaving(false);
    }
  };

  const startSync = async (platform: string) => {
    try {
      await api.startSync(platform);
      setSyncStarted((current) => Array.from(new Set([...current, platform])));
      message.info(`已开始同步 ${platformName(platform)}`);
    } catch (error) {
      message.error(errText(error));
    }
  };

  const finish = async () => {
    try {
      const state = await api.completeOnboarding();
      setOnboarding(state);
      await queryClient.invalidateQueries({ queryKey: ["bootstrap"] });
      message.success("配置完成，欢迎使用 ICPC 备赛工作站");
      navigate("/");
    } catch (error) {
      message.error(errText(error));
    }
  };

  const providerReady = (providersQuery.data?.length ?? 0) > 0;
  const syncDone = selected.every((p) => sync[p]?.status === "done");

  return (
    <div className="onboarding-wrap">
      <div className="onboarding-card">
        <div className="flex between center mb-14">
          <div>
            <Typography.Title level={3} style={{ margin: 0 }}>
              ICPC 备赛工作站
            </Typography.Title>
            <div className="muted">
              先接上你的 OJ 与 Agent，之后所有训练数据都在本机闭环。
            </div>
          </div>
          <Tag color="cyan">首次配置</Tag>
        </div>
        <Steps
          size="small"
          current={step}
          items={[
            { title: "开始" },
            { title: "平台" },
            { title: "登录" },
            { title: "Agent" },
            { title: "同步" },
          ]}
        />
        <Divider style={{ margin: "20px 0" }} />

        {step === 0 ? (
          <div>
            <Typography.Title level={4}>把训练、复盘、推荐放在一个工作台</Typography.Title>
            <Typography.Paragraph className="muted">
              这个应用会通过浏览器自动化读取你在 OJ 上的训练状态，结合 Agent
              生成推荐题目与报告；提交后会自动分析代码并刷新训练方案。你的 API Key
              存在系统钥匙串，OJ 登录态存在应用内浏览器。
            </Typography.Paragraph>
            <div className="prompt-grid">
              <div className="prompt-card">
                <div className="title">1. 选择 OJ</div>
                <div className="desc">Codeforces / AtCoder / 洛谷，可多选。</div>
              </div>
              <div className="prompt-card">
                <div className="title">2. 配置 Agent</div>
                <div className="desc">默认 OpenCode Go + DeepSeek V4.1 Flash。</div>
              </div>
              <div className="prompt-card">
                <div className="title">3. 首次同步</div>
                <div className="desc">自动读取 rating、提交与题池。</div>
              </div>
              <div className="prompt-card">
                <div className="title">4. 开始训练</div>
                <div className="desc">推荐题目、写代码、跑样例、提交、复盘。</div>
              </div>
            </div>
            <div className="mt-18 flex between center">
              <span className="muted tiny">数据仅保存在本机；不会上传到任何自建服务器。</span>
              <Button type="primary" size="large" onClick={() => persistStep(1)}>
                开始配置
              </Button>
            </div>
          </div>
        ) : null}

        {step === 1 ? (
          <div>
            <Typography.Title level={4}>你正在使用哪些 OJ？</Typography.Title>
            <Typography.Paragraph className="muted">
              至少选择一个。之后可以在设置里增加或移除。
            </Typography.Paragraph>
            <div className="card-grid" style={{ gridTemplateColumns: "1fr 1fr 1fr" }}>
              {(platformsQuery.data ?? []).map((platform) => {
                const meta = PLATFORM_META[platform.id];
                return (
                  <Card
                    key={platform.id}
                    size="small"
                    className="provider-card"
                    style={{
                      borderColor: selected.includes(platform.id)
                        ? "#2b9eb3"
                        : undefined,
                    }}
                    onClick={() => togglePlatform(platform.id, !selected.includes(platform.id))}
                  >
                    <div className="flex between center">
                      <strong>{platform.name}</strong>
                      <Checkbox checked={selected.includes(platform.id)} />
                    </div>
                    <div className="muted tiny mt-8">{meta?.desc ?? platform.description}</div>
                  </Card>
                );
              })}
            </div>
            <div className="mt-18 flex between center">
              <Button onClick={() => persistStep(0)}>上一步</Button>
              <Button type="primary" onClick={savePlatforms}>
                下一步：登录
              </Button>
            </div>
          </div>
        ) : null}

        {step === 2 ? (
          <div>
            <Typography.Title level={4}>登录 OJ 账号</Typography.Title>
            <Typography.Paragraph className="muted">
              点“打开登录页”会在应用内打开浏览器；登录后点“验证登录”。遇到验证码时直接在右侧页面完成即可。
            </Typography.Paragraph>
            <div className="card-grid">
              {selected.map((platform) => {
                const data = platformsQuery.data?.find((p) => p.id === platform);
                return (
                  <Card
                    key={platform}
                    size="small"
                    title={
                      <Space>
                        <span>{platformName(platform)}</span>
                        {data?.loggedIn ? (
                          <Tag color="green" icon={<CheckCircleOutlined />}>
                            已登录
                          </Tag>
                        ) : (
                          <Tag>未验证</Tag>
                        )}
                      </Space>
                    }
                  >
                    <Space orientation="vertical" style={{ width: "100%" }}>
                      <Space.Compact style={{ width: "100%" }}>
                        <Input
                          style={{ flex: 1 }}
                          value={handles[platform] ?? ""}
                          onChange={(event) =>
                            setHandles((current) => ({
                              ...current,
                              [platform]: event.target.value,
                            }))
                          }
                          placeholder={
                            platform === "luogu" ? "洛谷 UID（数字）" : "用户名 / handle"
                          }
                        />
                        <Button onClick={() => saveHandle(platform)}>保存</Button>
                      </Space.Compact>
                      <Space wrap>
                        <Button
                          icon={<LoginOutlined />}
                          onClick={() => openLogin(platform)}
                        >
                          打开登录页
                        </Button>
                        <Button onClick={() => verify(platform)}>验证登录</Button>
                      </Space>
                      <span className="muted tiny">
                        也可以先跳过登录，同步时再处理。账号信息只保存在本机。
                      </span>
                    </Space>
                  </Card>
                );
              })}
            </div>
            <div className="mt-18 flex between center">
              <Button onClick={() => persistStep(1)}>上一步</Button>
              <Space>
                <Button onClick={() => persistStep(3)}>跳过登录</Button>
                <Button type="primary" onClick={() => persistStep(3)}>
                  下一步：配置 Agent
                </Button>
              </Space>
            </div>
          </div>
        ) : null}

        {step === 3 ? (
          <div>
            <Typography.Title level={4}>配置内置 Agent</Typography.Title>
            <Typography.Paragraph className="muted">
              默认使用 OpenCode Go 的 <span className="mono">deepseek-v4.1-flash</span>
              （也可选 DeepSeek 官方 API）。四个角色（对话/推荐/分析/报告）会自动使用这个模型，之后可以分别修改。
            </Typography.Paragraph>
            {providerReady ? (
              <Alert
                type="success"
                showIcon
                style={{ marginBottom: 14 }}
                title="已检测到至少一个模型提供商"
                description="可以继续下一步，或在这里替换/新增提供商。"
              />
            ) : null}
            <div className="card-grid" style={{ gridTemplateColumns: "repeat(5, minmax(0, 1fr))" }}>
              {PROVIDERS.map((provider) => (
                <div
                  key={provider.kind}
                  className={`provider-card ${kind === provider.kind ? "active" : ""}`}
                  onClick={() => chooseProvider(provider.kind)}
                >
                  <div className="flex between center">
                    <strong>{provider.label}</strong>
                    {provider.badge ? <Tag color="cyan">{provider.badge}</Tag> : null}
                  </div>
                  <div className="muted tiny mt-8">{provider.desc}</div>
                </div>
              ))}
            </div>
            <div className="mt-18 card-grid" style={{ gridTemplateColumns: "1fr 1fr" }}>
              <div>
                <div className="muted tiny mb-8">Base URL</div>
                <Input
                  value={base}
                  onChange={(event) => setBase(event.target.value)}
                  placeholder="https://..."
                />
                <div className="muted tiny mt-12 mb-8">API Key</div>
                <Input.Password
                  value={apiKey}
                  onChange={(event) => setApiKey(event.target.value)}
                  placeholder={existingProvider?.hasKey ? "已保存，留空则不修改" : "sk-..."}
                />
                <div className="muted tiny mt-12 mb-8">默认模型</div>
                <AutoComplete
                  value={model}
                  onChange={setModel}
                  options={models.map((value) => ({ value }))}
                  placeholder={kind === "opencodego" ? "deepseek-v4.1-flash" : "模型 ID"}
                  style={{ width: "100%" }}
                />
                <div className="muted tiny mt-8">
                  {kind === "opencodego"
                    ? "DeepSeek V4.1 Flash 需要 OpenCode Console → Privacy → Regions 选择 Global。"
                    : "保存后会自动为四个角色使用该模型。"}
                </div>
              </div>
              <div>
                <div className="panel" style={{ padding: 14 }}>
                  <div className="muted tiny mb-8">连接测试</div>
                  <Space wrap>
                    <Button loading={testing} onClick={testProvider}>
                      测试连接
                    </Button>
                    <Button type="primary" loading={saving} onClick={saveProvider}>
                      保存并继续
                    </Button>
                  </Space>
                  {models.length ? (
                    <div className="mt-12">
                      <div className="muted tiny">发现 {models.length} 个模型：</div>
                      <div className="flex wrap gap-8 mt-8">
                        {models.slice(0, 12).map((id) => (
                          <Tag key={id} className="mono" style={{ cursor: "pointer" }} onClick={() => setModel(id)}>
                            {id}
                          </Tag>
                        ))}
                      </div>
                    </div>
                  ) : null}
                  <Divider style={{ margin: "14px 0" }} />
                  <div className="muted tiny">
                    API Key 存入 macOS 钥匙串；日志和数据库不保存明文。
                  </div>
                </div>
              </div>
            </div>
            <div className="mt-18 flex between center">
              <Button onClick={() => persistStep(2)}>上一步</Button>
              <Space>
                <Button onClick={() => persistStep(4)}>稍后配置</Button>
                <Button type="primary" onClick={() => persistStep(4)}>
                  下一步：首次同步
                </Button>
              </Space>
            </div>
          </div>
        ) : null}

        {step === 4 ? (
          <div>
            <Typography.Title level={4}>首次同步</Typography.Title>
            <Typography.Paragraph className="muted">
              同步会校验登录态并读取平台数据。洛谷可能弹出人机验证，遇到时在右侧自动化面板完成。
            </Typography.Paragraph>
            <div className="card-grid">
              {selected.map((platform) => {
                const progress = sync[platform];
                return (
                  <Card key={platform} size="small">
                    <div className="flex between center">
                      <Space>
                        <strong>{platformName(platform)}</strong>
                        {progress?.status === "done" ? <Tag color="green">完成</Tag> : null}
                        {progress?.status === "error" ? <Tag color="red">失败</Tag> : null}
                      </Space>
                      <Button
                        size="small"
                        icon={<CloudSyncOutlined />}
                        onClick={() => startSync(platform)}
                        loading={progress?.status === "running"}
                      >
                        同步
                      </Button>
                    </div>
                    {progress ? (
                      <div className="mt-8">
                        <Progress
                          percent={Math.round(progress.percent)}
                          size="small"
                          status={
                            progress.status === "error"
                              ? "exception"
                              : progress.status === "done"
                                ? "success"
                                : "active"
                          }
                        />
                        <div className="muted tiny">{progress.message}</div>
                      </div>
                    ) : null}
                  </Card>
                );
              })}
            </div>
            <div className="mt-18 flex between center">
              <Button onClick={() => persistStep(3)}>上一步</Button>
              <Space>
                <Button onClick={finish}>跳过同步，进入工作站</Button>
                <Button type="primary" onClick={finish}>
                  {syncDone ? "完成" : "进入工作站"}
                </Button>
              </Space>
            </div>
            <div className="muted tiny mt-12">
              同步在后台进行；进入工作站后也可以在数据面板重新同步。
            </div>
          </div>
        ) : null}

        <div className="muted tiny" style={{ marginTop: 20 }}>
          你可以随时在设置中重新运行引导。当前步骤 {step + 1}/5
        </div>
      </div>
    </div>
  );
}
