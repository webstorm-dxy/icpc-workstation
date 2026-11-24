import { useEffect } from "react";
import { App as AntApp, ConfigProvider, Spin, theme } from "antd";
import zhCN from "antd/locale/zh_CN";
import { Navigate, Route, Routes, useLocation } from "react-router-dom";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { api, events } from "./lib/api";
import { useAppStore } from "./store";
import Shell from "./layout/Shell";
import ChatPage from "./pages/ChatPage";
import DashboardPage from "./pages/DashboardPage";
import OnboardingPage from "./pages/OnboardingPage";
import ProblemPage from "./pages/ProblemPage";
import PrintReportPage from "./pages/PrintReportPage";
import ReportsPage from "./pages/ReportsPage";
import SettingsPage from "./pages/SettingsPage";
import TrainingPage from "./pages/TrainingPage";
import AutomationDrawer from "./components/AutomationDrawer";
import PlanAgentDrawer from "./components/PlanAgentDrawer";
import ConfirmToolModal from "./components/ConfirmToolModal";
import { openUrl } from "@tauri-apps/plugin-opener";
import { errText } from "./lib/api";

function useBootstrap() {
  const query = useQuery({
    queryKey: ["bootstrap"],
    queryFn: async () => {
      const [onboarding, settings] = await Promise.all([
        api.getOnboardingState(),
        api.getSettings(),
      ]);
      return { onboarding, settings };
    },
  });
  const setOnboarding = useAppStore((s) => s.setOnboarding);
  const setSettings = useAppStore((s) => s.setSettings);
  useEffect(() => {
    if (query.data) {
      setOnboarding(query.data.onboarding);
      setSettings(query.data.settings);
    }
  }, [query.data, setOnboarding, setSettings]);
  return query;
}

function useAppEvents() {
  const queryClient = useQueryClient();
  const { message } = AntApp.useApp();
  const setAutomationState = useAppStore((s) => s.setAutomationState);
  const setAutomationOpen = useAppStore((s) => s.setAutomationOpen);
  const setConfirmation = useAppStore((s) => s.setConfirmation);
  const setSync = useAppStore((s) => s.setSync);
  const applyPlanAgent = useAppStore((s) => s.applyPlanAgent);

  useEffect(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];
    const register = async () => {
      const unlistenersOrVoid = await Promise.all([
        events.onAutomationState((event) => {
          setAutomationState({
            platform: event.platform,
            visible: true,
            status: event.status,
            message: event.message,
            needsHuman: event.needsHuman,
            hint: event.hint,
            url: event.url,
          });
          if (event.needsHuman || event.status !== "idle") {
            setAutomationOpen(true);
          }
        }),
        events.onAutomationExternal((event) => {
          openUrl(event.url).catch(() => {});
        }),
        events.onSyncProgress((progress) => {
          setSync(progress);
          if (progress.status === "done") {
            message.success(progress.message);
            queryClient.invalidateQueries({ queryKey: ["dashboard"] });
            queryClient.invalidateQueries({ queryKey: ["platforms"] });
            queryClient.invalidateQueries({ queryKey: ["plan"] });
          }
          if (progress.status === "error") {
            message.error(progress.message);
          }
        }),
        events.onAgentConfirm((confirmation) => {
          setConfirmation(confirmation);
        }),
        events.onAnalysisUpdated(() => {
          queryClient.invalidateQueries({ queryKey: ["dashboard"] });
          queryClient.invalidateQueries({ queryKey: ["submissions"] });
          queryClient.invalidateQueries({ queryKey: ["plan"] });
        }),
        events.onPlanUpdated(() => {
          queryClient.invalidateQueries({ queryKey: ["plan"] });
          queryClient.invalidateQueries({ queryKey: ["dashboard"] });
        }),
        events.onPlanAgent((event) => {
          applyPlanAgent(event);
        }),
        events.onPlanError((event) => {
          message.error(event.message);
          applyPlanAgent({
            phase: "error",
            trigger: event.trigger ?? "auto",
            message: event.message,
          });
        }),
        events.onSubmissionUpdated(() => {
          queryClient.invalidateQueries({ queryKey: ["dashboard"] });
          queryClient.invalidateQueries({ queryKey: ["submissions"] });
        }),
      ]);
      if (disposed) {
        unlistenersOrVoid.forEach((fn) => fn());
        return;
      }
      unlistenersOrVoid.forEach((fn) => {
        if (typeof fn === "function") unlisteners.push(fn);
      });
    };
    register().catch((error) => {
      console.warn("register event listeners failed", errText(error));
    });
    return () => {
      disposed = true;
      unlisteners.forEach((fn) => fn());
    };
  }, [
    message,
    queryClient,
    setAutomationOpen,
    setAutomationState,
    applyPlanAgent,
    setConfirmation,
    setSync,
  ]);
}

export default function App() {
  const bootstrap = useBootstrap();
  const settings = useAppStore((s) => s.settings);
  const onboarding = useAppStore((s) => s.onboarding);
  const location = useLocation();
  useAppEvents();

  const algorithm = settings?.theme === "light" ? theme.defaultAlgorithm : theme.darkAlgorithm;

  if (bootstrap.isLoading) {
    return (
      <div style={{ height: "100vh", display: "grid", placeItems: "center" }}>
        <Spin size="large" />
      </div>
    );
  }

  if (bootstrap.isError) {
    return (
      <div style={{ height: "100vh", display: "grid", placeItems: "center", padding: 40 }}>
        <div>
          <h2>应用启动失败</h2>
          <p className="muted">{errText(bootstrap.error)}</p>
        </div>
      </div>
    );
  }

  const onOnboarding = location.pathname.startsWith("/onboarding");
  if (onboarding && !onboarding.complete && !onOnboarding) {
    return <Navigate to="/onboarding" replace />;
  }

  return (
    <ConfigProvider
      locale={zhCN}
      theme={{
        algorithm,
        token: {
          colorPrimary: "#22d3ee",
          colorInfo: "#22d3ee",
          colorSuccess: "#34d399",
          colorWarning: "#fbbf24",
          colorError: "#f87171",
          colorBgBase: settings?.theme === "light" ? "#f7f9fb" : "#0b0f14",
          colorBgContainer: settings?.theme === "light" ? "#ffffff" : "#11161d",
          colorBorder: "#223041",
          borderRadius: 9,
          fontFamily:
            '-apple-system, BlinkMacSystemFont, "Segoe UI", "PingFang SC", "Microsoft YaHei", sans-serif',
        },
        components: {
          Layout: {
            siderBg: "#0d1219",
            bodyBg: "#0b0f14",
            headerBg: "#0b0f14",
          },
          Menu: {
            darkItemBg: "transparent",
            darkItemSelectedBg: "rgba(34, 211, 238, 0.12)",
            darkItemSelectedColor: "#67e8f9",
          },
        },
      }}
    >
      <AntApp>
        <Routes>
          <Route path="/onboarding" element={<OnboardingPage />} />
          <Route path="/print/report" element={<PrintReportPage />} />
          <Route element={<Shell />}>
            <Route path="/" element={<ChatPage />} />
            <Route path="/dashboard" element={<DashboardPage />} />
            <Route path="/training" element={<TrainingPage />} />
            <Route path="/problem/:problemId" element={<ProblemPage />} />
            <Route path="/reports" element={<ReportsPage />} />
            <Route path="/settings" element={<SettingsPage />} />
          </Route>
          <Route path="*" element={<Navigate to="/" replace />} />
        </Routes>
        <AutomationDrawer />
        <ConfirmToolModal />
      </AntApp>
    </ConfigProvider>
  );
}
