import { create } from "zustand";
import type {
  AutomationState,
  OnboardingState,
  PlanAgentEvent,
  PlanAgentState,
  Settings,
  SubmitResult,
  SyncProgress,
  ToolConfirmation,
} from "./types";

interface AppStore {
  onboarding: OnboardingState | null;
  settings: Settings | null;
  automation: AutomationState | null;
  automationOpen: boolean;
  automationRequest: { platform: string; url: string } | null;
  automationContext: {
    problemId: string;
    code: string;
    language?: string;
    since?: number;
  } | null;
  confirmation: ToolConfirmation | null;
  planAgent: PlanAgentState | null;
  sync: Record<string, SyncProgress>;
  submitResult: SubmitResult | null;
  setOnboarding: (value: OnboardingState | null) => void;
  setSettings: (value: Settings | null) => void;
  setAutomationState: (value: AutomationState | null) => void;
  setAutomationOpen: (open: boolean) => void;
  requestAutomation: (platform: string, url: string) => void;
  clearAutomationRequest: () => void;
  setAutomationContext: (
    value: AppStore["automationContext"],
  ) => void;
  setConfirmation: (value: ToolConfirmation | null) => void;
  openPlanAgent: () => void;
  applyPlanAgent: (event: PlanAgentEvent) => void;
  closePlanAgent: () => void;
  setSync: (value: SyncProgress) => void;
  clearSync: (platform: string) => void;
  setSubmitResult: (value: SubmitResult | null) => void;
}

export const useAppStore = create<AppStore>((set) => ({
  onboarding: null,
  settings: null,
  automation: null,
  automationOpen: false,
  automationRequest: null,
  automationContext: null,
  confirmation: null,
  planAgent: null,
  sync: {},
  submitResult: null,
  setOnboarding: (value) => set({ onboarding: value }),
  setSettings: (value) => set({ settings: value }),
  setAutomationState: (value) => set({ automation: value }),
  setAutomationOpen: (open) => set({ automationOpen: open }),
  requestAutomation: (platform, url) =>
    set({
      automationRequest: { platform, url },
      automationOpen: true,
    }),
  clearAutomationRequest: () => set({ automationRequest: null }),
  setAutomationContext: (value) => set({ automationContext: value }),
  setConfirmation: (value) => set({ confirmation: value }),
  openPlanAgent: () =>
    set({
      planAgent: {
        open: true,
        phase: "start",
        output: "",
        status: "准备分析最近提交与代码…",
        error: null,
      },
    }),
  applyPlanAgent: (event) =>
    set((state) => {
      const current = state.planAgent;
      if (event.phase === "start") {
        return {
          planAgent: {
            open: true,
            phase: "start",
            output: "",
            status: event.message ?? "正在分析提交与代码…",
            error: null,
            provider: event.provider ?? current?.provider,
            model: event.model ?? current?.model,
          },
        };
      }
      if (!current?.open) return {};
      switch (event.phase) {
        case "model":
          return {
            planAgent: {
              ...current,
              phase: "model",
              status: event.message ?? "Agent 正在分析提交与代码…",
              provider: event.provider ?? current.provider,
              model: event.model ?? current.model,
            },
          };
        case "output":
          return {
            planAgent: {
              ...current,
              phase: "output",
              status: "Agent 正在输出分析与推荐…",
              output: (current.output + (event.delta ?? "")).slice(0, 300_000),
            },
          };
        case "done":
          return {
            planAgent: {
              ...current,
              phase: "done",
              status: event.message ?? "训练计划已生成",
              planId: event.planId,
            },
          };
        case "error":
          return {
            planAgent: {
              ...current,
              phase: "error",
              status: "生成失败",
              error: event.message ?? "Agent 生成失败",
            },
          };
        default:
          return {};
      }
    }),
  closePlanAgent: () => set({ planAgent: null }),
  setSync: (value) =>
    set((state) => ({ sync: { ...state.sync, [value.platform]: value } })),
  clearSync: (platform) =>
    set((state) => {
      const next = { ...state.sync };
      delete next[platform];
      return { sync: next };
    }),
  setSubmitResult: (value) => set({ submitResult: value }),
}));
