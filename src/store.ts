import { create } from "zustand";
import type {
  AutomationState,
  OnboardingState,
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
