import {
  Alert,
  App,
  AutoComplete,
  Button,
  Card,
  Divider,
  Input,
  InputNumber,
  Modal,
  Progress,
  Select,
  Space,
  Switch,
  Tabs,
  Tag,
  Typography,
} from "antd";
import {
  CloudSyncOutlined,
  DeleteOutlined,
  LoginOutlined,
  PlusOutlined,
  ReloadOutlined,
  SettingOutlined,
} from "@ant-design/icons";
import { useEffect, useMemo, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "react-router-dom";
import { api, errText } from "../lib/api";
import {
  platformName,
  roleLabel,
  verdictLabel,
} from "../lib/format";
import { useAppStore } from "../store";
import type {
  Platform,
  Provider,
  ProviderUsage,
  RoleModel,
  Settings,
  Toolchain,
} from "../types";

const PROVIDER_PRESETS: Record<
  string,
  { label: string; base: string; model: string }
> = {
  opencodego: {
    label: "OpenCode Go",
    base: "https://opencode.ai/inference/go",
    model: "deepseek-v4.1-flash",
  },
  deepseek: {
    label: "DeepSeek",
    base: "https://api.deepseek.com",
    model: "deepseek-flash",
  },
  openai: { label: "OpenAI", base: "https://api.openai.com/v1", model: "" },
  anthropic: {
    label: "Anthropic",
    base: "https://api.anthropic.com/v1",
    model: "",
  },
  custom: { label: "自定义 OpenAI 兼容", base: "", model: "" },
};

const ROLE_ORDER = ["chat", "recommend", "analyze", "report"];

function RoleEditor({
  role,
  providers,
}: {
  role: RoleModel;
  providers: Provider[];
}) {
  const { message } = App.useApp();
  const queryClient = useQueryClient();
  const [providerId, setProviderId] = useState(role.providerId ?? "");
  const [modelId, setModelId] = useState(role.modelId ?? "");
  const [saving, setSaving] = useState(false);
  const models = useQuery({
    queryKey: ["models", providerId],
    queryFn: () => (providerId ? api.listModels(providerId) : Promise.resolve([])),
    enabled: !!providerId,
    retry: 0,
  });

  useEffect(() => {
    if (!models.data?.length) return;
    if (!models.data.some((model) => model.id === modelId)) {
      setModelId(models.data[0].id);
    }
  }, [models.data, modelId]);

  const save = async () => {
    if (!providerId || !modelId) {
      message.warning("请选择提供商与模型");
      return;
    }
    setSaving(true);
    try {
      await api.setRoleModel({ role: role.role, providerId, modelId });
      await queryClient.invalidateQueries({ queryKey: ["role-models"] });
      message.success(`${roleLabel(role.role)}角色已更新`);
    } catch (error) {
      message.error(errText(error));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Card
      size="small"
      title={roleLabel(role.role)}
      extra={
        <span className="muted tiny">
          {role.providerName ?? "未配置"} / {role.modelId ?? "—"}
        </span>
      }
    >
      <Space orientation="vertical" style={{ width: "100%" }} size={8}>
        <Select
          value={providerId || undefined}
          onChange={setProviderId}
          placeholder="选择提供商"
          style={{ width: "100%" }}
          options={providers.map((provider) => ({
            value: provider.id,
            label: `${provider.name} · ${provider.kind}`,
          }))}
        />
        <AutoComplete
          value={modelId}
          onChange={setModelId}
          options={(models.data ?? []).map((model) => ({
            value: model.id,
            label: model.id,
          }))}
          placeholder={models.isLoading ? "加载模型…" : "模型 ID"}
          style={{ width: "100%" }}
        />
        <div className="flex between center">
          <span className="muted tiny">
            {models.isError ? errText(models.error) : ""}
          </span>
          <Button size="small" type="primary" loading={saving} onClick={save}>
            保存
          </Button>
        </div>
      </Space>
    </Card>
  );
}

function PlatformCard({ platform }: { platform: Platform }) {
  const { message } = App.useApp();
  const queryClient = useQueryClient();
  const requestAutomation = useAppStore((s) => s.requestAutomation);
  const sync = useAppStore((s) => s.sync);
  const [handle, setHandle] = useState(platform.handle ?? "");
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    setHandle(platform.handle ?? "");
  }, [platform.handle]);

  const save = async () => {
    setSaving(true);
    try {
      await api.savePlatformAccount(platform.id, handle);
      await queryClient.invalidateQueries({ queryKey: ["platforms"] });
      message.success(`${platformName(platform.id)} 账号已保存`);
    } catch (error) {
      message.error(errText(error));
    } finally {
      setSaving(false);
    }
  };

  const verify = async () => {
    try {
      const result = await api.verifyPlatformLogin(platform.id);
      if (result.loggedIn) message.success(result.message);
      else message.warning(result.message);
      await queryClient.invalidateQueries({ queryKey: ["platforms"] });
    } catch (error) {
      message.error(errText(error));
    }
  };

  return (
    <Card
      size="small"
      title={
        <Space>
          <span>{platform.name}</span>
          {platform.loggedIn ? (
            <Tag color="green">已登录</Tag>
          ) : (
            <Tag>未验证</Tag>
          )}
        </Space>
      }
      extra={
        <Button
          size="small"
          icon={<CloudSyncOutlined />}
          loading={sync[platform.id]?.status === "running"}
          onClick={() => api.startSync(platform.id).catch((error) => message.error(errText(error)))}
        >
          同步
        </Button>
      }
    >
      <Space orientation="vertical" style={{ width: "100%" }} size={8}>
        <div className="muted tiny">{platform.description}</div>
        <Space.Compact style={{ width: "100%" }}>
          <Input
            style={{ flex: 1 }}
            value={handle}
            onChange={(event) => setHandle(event.target.value)}
            placeholder={platform.handleLabel}
          />
          <Button loading={saving} onClick={save}>
            保存
          </Button>
        </Space.Compact>
        <Space wrap>
          <Button
            size="small"
            icon={<LoginOutlined />}
            onClick={() => requestAutomation(platform.id, platform.loginUrl)}
          >
            打开登录页
          </Button>
          <Button size="small" onClick={verify}>
            验证登录
          </Button>
        </Space>
        {sync[platform.id] ? (
          <div>
            <Progress
              percent={Math.round(sync[platform.id].percent)}
              size="small"
              status={
                sync[platform.id].status === "error"
                  ? "exception"
                  : sync[platform.id].status === "done"
                    ? "success"
                    : "active"
              }
            />
            <div className="muted tiny">{sync[platform.id].message}</div>
          </div>
        ) : null}
      </Space>
    </Card>
  );
}

export default function SettingsPage() {
  const { message } = App.useApp();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const settings = useAppStore((s) => s.settings);
  const setSettings = useAppStore((s) => s.setSettings);
  const setOnboarding = useAppStore((s) => s.setOnboarding);
  const onboarding = useAppStore((s) => s.onboarding);
  const providers = useQuery({ queryKey: ["providers"], queryFn: api.listProviders });
  const roles = useQuery({ queryKey: ["role-models"], queryFn: api.getRoleModels });
  const platforms = useQuery({ queryKey: ["platforms"], queryFn: api.listPlatforms });
  const toolchains = useQuery({ queryKey: ["toolchains"], queryFn: api.detectToolchains });
  const appInfo = useQuery({ queryKey: ["app-info"], queryFn: api.getAppInfo });

  const [providerModal, setProviderModal] = useState(false);
  const [editing, setEditing] = useState<Provider | null>(null);
  const [kind, setKind] = useState("opencodego");
  const [baseUrl, setBaseUrl] = useState(PROVIDER_PRESETS.opencodego.base);
  const [name, setName] = useState(PROVIDER_PRESETS.opencodego.label);
  const [apiKey, setApiKey] = useState("");
  const [model, setModel] = useState(PROVIDER_PRESETS.opencodego.model);
  const [models, setModels] = useState<string[]>([]);
  const [testing, setTesting] = useState(false);
  const [saving, setSaving] = useState(false);
  const [usage, setUsage] = useState<ProviderUsage | null>(null);
  const [usageOpen, setUsageOpen] = useState(false);

  const currentSettings = settings ?? null;

  const patchSettings = async (patch: Partial<Settings>) => {
    try {
      const next = await api.updateSettings(patch);
      setSettings(next);
      await queryClient.invalidateQueries({ queryKey: ["bootstrap"] });
    } catch (error) {
      message.error(errText(error));
    }
  };

  const openNewProvider = () => {
    setEditing(null);
    setKind("opencodego");
    setBaseUrl(PROVIDER_PRESETS.opencodego.base);
    setName(PROVIDER_PRESETS.opencodego.label);
    setApiKey("");
    setModel(PROVIDER_PRESETS.opencodego.model);
    setModels([]);
    setProviderModal(true);
  };

  const openEditProvider = (provider: Provider) => {
    setEditing(provider);
    setKind(provider.kind);
    setBaseUrl(provider.baseUrl);
    setName(provider.name);
    setApiKey("");
    setModel(String(provider.config?.default_model ?? ""));
    setModels([]);
    setProviderModal(true);
  };

  const chooseKind = (nextKind: string) => {
    const preset = PROVIDER_PRESETS[nextKind];
    setKind(nextKind);
    if (preset) {
      setBaseUrl(preset.base);
      setName(preset.label);
      if (!model) setModel(preset.model);
    }
  };

  const test = async () => {
    if (!baseUrl.trim()) {
      message.warning("请填写 Base URL");
      return;
    }
    setTesting(true);
    try {
      const result = await api.testProvider({
        id: editing?.id,
        kind,
        name,
        baseUrl: baseUrl.trim(),
        apiKey: apiKey.trim() || undefined,
        config: { default_model: model.trim() },
      });
      if (result.ok) {
        setModels(result.models.map((item) => item.id));
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
    if (!apiKey.trim() && !editing?.hasKey) {
      message.warning("请填写 API Key");
      return;
    }
    setSaving(true);
    try {
      await api.upsertProvider({
        id: editing?.id,
        kind,
        name: name.trim() || PROVIDER_PRESETS[kind]?.label || kind,
        baseUrl: baseUrl.trim(),
        apiKey: apiKey.trim() || undefined,
        config: { default_model: model.trim() },
      });
      await queryClient.invalidateQueries({ queryKey: ["providers"] });
      await queryClient.invalidateQueries({ queryKey: ["role-models"] });
      await queryClient.invalidateQueries({ queryKey: ["bootstrap"] });
      setProviderModal(false);
      message.success("提供商已保存");
    } catch (error) {
      message.error(errText(error));
    } finally {
      setSaving(false);
    }
  };

  const deleteProvider = (provider: Provider) => {
    Modal.confirm({
      title: `删除 ${provider.name}？`,
      content: "API Key 会从系统钥匙串删除；依赖该提供商的角色会失效。",
      okText: "删除",
      okButtonProps: { danger: true },
      cancelText: "取消",
      onOk: async () => {
        try {
          await api.deleteProvider(provider.id);
          await queryClient.invalidateQueries({ queryKey: ["providers"] });
          await queryClient.invalidateQueries({ queryKey: ["role-models"] });
          message.success("已删除");
        } catch (error) {
          message.error(errText(error));
        }
      },
    });
  };

  const showUsage = async (provider: Provider) => {
    try {
      const result = await api.getProviderUsage(provider.id);
      setUsage(result);
      setUsageOpen(true);
    } catch (error) {
      message.error(errText(error));
    }
  };

  const reset = () => {
    Modal.confirm({
      title: "清空所有本地数据？",
      content:
        "会删除提供商与 Key、平台账号、题目/提交/分析/计划/报告/对话与设置。此操作不可恢复。",
      okText: "清空数据",
      okButtonProps: { danger: true },
      cancelText: "取消",
      onOk: async () => {
        try {
          await api.resetData();
          window.location.reload();
        } catch (error) {
          message.error(errText(error));
        }
      },
    });
  };

  const restartOnboarding = async () => {
    try {
      await api.setOnboardingStep(0);
      if (onboarding) {
        setOnboarding({ ...onboarding, complete: false, step: 0 });
      }
      navigate("/onboarding");
    } catch (error) {
      message.error(errText(error));
    }
  };

  const providerTab = (
    <div>
      <div className="flex between center mb-14">
        <Typography.Paragraph className="muted" style={{ margin: 0 }}>
          API Key 存入系统钥匙串。OpenCode Go 默认模型为{" "}
          <span className="mono">deepseek-v4.1-flash</span>。
        </Typography.Paragraph>
        <Button type="primary" icon={<PlusOutlined />} onClick={openNewProvider}>
          添加提供商
        </Button>
      </div>
      <div className="card-grid" style={{ gridTemplateColumns: "repeat(2, minmax(0, 1fr))" }}>
        {(providers.data ?? []).map((provider) => (
          <Card
            key={provider.id}
            size="small"
            title={
              <Space>
                <span>{provider.name}</span>
                <Tag color="blue">{provider.kind}</Tag>
                {provider.hasKey ? (
                  <Tag color="green">Key 已保存</Tag>
                ) : (
                  <Tag color="orange">缺少 Key</Tag>
                )}
              </Space>
            }
            extra={
              <Space>
                {provider.kind === "opencodego" ? (
                  <Button size="small" onClick={() => showUsage(provider)}>
                    用量
                  </Button>
                ) : null}
                <Button size="small" onClick={() => openEditProvider(provider)}>
                  编辑
                </Button>
                <Button
                  size="small"
                  danger
                  icon={<DeleteOutlined />}
                  onClick={() => deleteProvider(provider)}
                />
              </Space>
            }
          >
            <div className="muted tiny mono">{provider.baseUrl}</div>
            <div className="muted tiny mt-8">
              默认模型：{String(provider.config?.default_model ?? "未设置")}
            </div>
          </Card>
        ))}
      </div>
      {!providers.data?.length ? (
        <Alert
          type="warning"
          showIcon
          title="还没有配置模型提供商"
          description="没有模型时，Agent 对话、计划生成与代码分析都不可用。"
        />
      ) : null}
    </div>
  );

  const roleTab = (
    <div className="card-grid" style={{ gridTemplateColumns: "repeat(2, minmax(0, 1fr))" }}>
      {ROLE_ORDER.map((role) => {
        const current = roles.data?.find((item) => item.role === role);
        if (!current) {
          return (
            <Card key={role} size="small" title={roleLabel(role)}>
              <span className="muted tiny">加载中…</span>
            </Card>
          );
        }
        return (
          <RoleEditor
            key={role}
            role={current}
            providers={providers.data ?? []}
          />
        );
      })}
    </div>
  );

  const platformTab = (
    <div className="card-grid" style={{ gridTemplateColumns: "repeat(2, minmax(0, 1fr))" }}>
      {(platforms.data ?? []).map((platform) => (
        <PlatformCard key={platform.id} platform={platform} />
      ))}
    </div>
  );

  const editorTab = (
    <div className="card-grid" style={{ gridTemplateColumns: "1fr 1fr" }}>
      <Card size="small" title="C++ 工具链">
        <Space orientation="vertical" style={{ width: "100%" }}>
          <div className="muted tiny">
            默认优先 GNU g++（本机检测到 Homebrew g++-16 时会自动选择）。只有 clang 时使用内置
            bits 兼容头。
          </div>
          <Button
            icon={<ReloadOutlined />}
            onClick={() => toolchains.refetch()}
            loading={toolchains.isFetching}
          >
            重新探测编译器
          </Button>
          <Select
            value={currentSettings?.compilerPath ?? undefined}
            onChange={(value) => patchSettings({ compilerPath: value })}
            placeholder="使用自动探测"
            style={{ width: "100%" }}
            options={(toolchains.data ?? []).map((tool: Toolchain) => ({
              value: tool.path,
              label: `${tool.path} · ${tool.kind}${tool.supportsBits ? " · bits ✓" : ""}`,
            }))}
          />
          <Select
            value={currentSettings?.compilerStandard ?? "gnu++17"}
            onChange={(value) => patchSettings({ compilerStandard: value })}
            style={{ width: "100%" }}
            options={[
              { value: "gnu++17", label: "GNU++17（ICPC 默认）" },
              { value: "gnu++20", label: "GNU++20" },
              { value: "gnu++23", label: "GNU++23" },
            ]}
          />
        </Space>
      </Card>
      <Card size="small" title="检测到的编译器">
        <Space orientation="vertical" style={{ width: "100%" }}>
          {(toolchains.data ?? []).map((tool) => (
            <div key={tool.path} className="flex between center gap-8">
              <span className="mono tiny" style={{ overflow: "hidden" }}>
                {tool.path}
              </span>
              <Space>
                <Tag>{tool.kind}</Tag>
                <Tag color={tool.supportsBits ? "green" : "default"}>
                  {tool.supportsBits ? "bits ✓" : "bits ×"}
                </Tag>
              </Space>
            </div>
          ))}
          {!toolchains.data?.length ? (
            <span className="muted tiny">没有找到编译器，请安装 Xcode CLT 或 Homebrew gcc。</span>
          ) : null}
        </Space>
      </Card>
    </div>
  );

  const automationTab = (
    <div className="card-grid" style={{ gridTemplateColumns: "1fr 1fr" }}>
      <Card size="small" title="提交">
        <Space orientation="vertical" style={{ width: "100%" }}>
          <div className="flex between center">
            <span>默认自动点击提交</span>
            <Switch
              checked={currentSettings?.autoSubmit ?? false}
              onChange={(checked) => patchSettings({ autoSubmit: checked })}
            />
          </div>
          <div className="flex between center">
            <span>提交前必须人工确认</span>
            <Switch
              checked={currentSettings?.requireSubmitConfirm ?? true}
              onChange={(checked) => patchSettings({ requireSubmitConfirm: checked })}
            />
          </div>
          <div className="muted tiny">
            首次使用建议开启“必须人工确认”；验证码/风控会暂停自动化并等待你操作。
          </div>
        </Space>
      </Card>
      <Card size="small" title="分析与同步">
        <Space orientation="vertical" style={{ width: "100%" }}>
          <div className="flex between center">
            <span>允许把代码发给模型做分析</span>
            <Switch
              checked={currentSettings?.allowCodeUpload ?? true}
              onChange={(checked) => patchSettings({ allowCodeUpload: checked })}
            />
          </div>
          <div className="flex between center">
            <span>提交后自动刷新训练方案</span>
            <Switch
              checked={currentSettings?.autoRefreshPlan ?? true}
              onChange={(checked) => patchSettings({ autoRefreshPlan: checked })}
            />
          </div>
          <div className="flex between center gap-8">
            <span>洛谷同步最近提交数</span>
            <InputNumber
              min={50}
              max={2000}
              step={50}
              value={currentSettings?.luoguSyncDepth ?? 500}
              onChange={(value) => {
                if (typeof value === "number") patchSettings({ luoguSyncDepth: value });
              }}
            />
          </div>
        </Space>
      </Card>
    </div>
  );

  const dataTab = (
    <div className="card-grid" style={{ gridTemplateColumns: "1fr 1fr" }}>
      <Card size="small" title="本地数据">
        <Space orientation="vertical" style={{ width: "100%" }}>
          <div className="muted tiny">版本 {appInfo.data?.version ?? "—"}</div>
          <div className="muted tiny mono" style={{ wordBreak: "break-all" }}>
            {appInfo.data?.dataDir ?? "—"}
          </div>
          <Divider style={{ margin: "8px 0" }} />
          <Space wrap>
            <Button onClick={restartOnboarding} icon={<SettingOutlined />}>
              重新运行引导
            </Button>
            <Button
              onClick={async () => {
                try {
                  message.loading({ content: "写入演示数据…", key: "seed2" });
                  await api.seedDemoData();
                  await queryClient.invalidateQueries();
                  message.success({ content: "演示数据已写入", key: "seed2" });
                } catch (error) {
                  message.error({ content: errText(error), key: "seed2" });
                }
              }}
            >
              写入演示数据
            </Button>
          </Space>
        </Space>
      </Card>
      <Card size="small" title="危险操作">
        <Space orientation="vertical" style={{ width: "100%" }}>
          <div className="muted tiny">
            清空所有提供商、Key、平台账号、题目、提交、分析、计划、报告与对话。不可恢复。
          </div>
          <Button danger onClick={reset}>
            清空所有本地数据
          </Button>
        </Space>
      </Card>
    </div>
  );

  return (
    <div className="page">
      <div className="page-header">
        <div>
          <h1 className="page-title">设置</h1>
          <div className="page-subtitle">
            提供商、角色、OJ、编译器与自动化策略都在这里配置。
          </div>
        </div>
      </div>
      <Tabs
        items={[
          { key: "providers", label: "模型提供商", children: providerTab },
          { key: "roles", label: "Agent 角色", children: roleTab },
          { key: "platforms", label: "OJ 平台", children: platformTab },
          { key: "editor", label: "编辑器与编译器", children: editorTab },
          { key: "automation", label: "自动化与分析", children: automationTab },
          { key: "data", label: "数据", children: dataTab },
        ]}
      />

      <Modal
        open={providerModal}
        title={editing ? "编辑提供商" : "添加提供商"}
        onCancel={() => setProviderModal(false)}
        onOk={saveProvider}
        confirmLoading={saving}
        okText="保存"
        cancelText="取消"
        width={620}
      >
        <Space orientation="vertical" style={{ width: "100%" }} size={10}>
          <div>
            <div className="muted tiny mb-8">类型</div>
            <Select
              value={kind}
              onChange={chooseKind}
              style={{ width: "100%" }}
              options={Object.entries(PROVIDER_PRESETS).map(([value, preset]) => ({
                value,
                label: preset.label,
              }))}
            />
          </div>
          <div>
            <div className="muted tiny mb-8">名称</div>
            <Input value={name} onChange={(event) => setName(event.target.value)} />
          </div>
          <div>
            <div className="muted tiny mb-8">Base URL</div>
            <Input value={baseUrl} onChange={(event) => setBaseUrl(event.target.value)} />
          </div>
          <div>
            <div className="muted tiny mb-8">API Key</div>
            <Input.Password
              value={apiKey}
              onChange={(event) => setApiKey(event.target.value)}
              placeholder={editing?.hasKey ? "已保存，留空不修改" : "sk-..."}
            />
          </div>
          <div>
            <div className="muted tiny mb-8">默认模型</div>
            <AutoComplete
              value={model}
              onChange={setModel}
              options={models.map((value) => ({ value }))}
              style={{ width: "100%" }}
              placeholder="deepseek-v4.1-flash / deepseek-flash"
            />
          </div>
          <Space wrap>
            <Button loading={testing} onClick={test}>
              测试连接
            </Button>
            {models.length ? (
              <span className="muted tiny">发现 {models.length} 个模型</span>
            ) : null}
          </Space>
          {kind === "opencodego" ? (
            <Alert
              type="info"
              showIcon
              title="OpenCode Go 提示"
              description="DeepSeek V4.1 Flash 需要在 OpenCode Console → Privacy → Regions 选择 Global；请求会自动携带稳定的 x-opencode-session。"
            />
          ) : null}
        </Space>
      </Modal>

      <Modal
        open={usageOpen}
        title="OpenCode Go 用量"
        footer={null}
        onCancel={() => setUsageOpen(false)}
      >
        {usage?.meters?.length ? (
          <Space orientation="vertical" style={{ width: "100%" }}>
            {usage.meters.map((meter) => (
              <div key={meter.window}>
                <div className="flex between center">
                  <span>
                    {meter.window === "rolling"
                      ? "5 小时窗口"
                      : meter.window === "weekly"
                        ? "每周"
                        : "每月"}
                  </span>
                  <Tag color={meter.status === "ok" ? "green" : "red"}>
                    {meter.percent}%
                  </Tag>
                </div>
                <Progress
                  percent={meter.percent}
                  size="small"
                  status={meter.status === "ok" ? "normal" : "exception"}
                />
                {meter.resetsAt ? (
                  <div className="muted tiny">重置：{meter.resetsAt}</div>
                ) : null}
              </div>
            ))}
          </Space>
        ) : (
          <div className="muted">{usage?.message ?? "暂无用量数据"}</div>
        )}
      </Modal>
    </div>
  );
}
