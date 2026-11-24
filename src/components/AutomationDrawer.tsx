import { App, Alert, Button, Space, Tag } from "antd";
import { CloseOutlined, ReloadOutlined } from "@ant-design/icons";
import { useCallback, useEffect, useRef } from "react";
import { api, errText } from "../lib/api";
import { automationStatusLabel, platformName, statusColor } from "../lib/format";
import { useAppStore } from "../store";

export default function AutomationDrawer() {
  const { message } = App.useApp();
  const automation = useAppStore((s) => s.automation);
  const automationOpen = useAppStore((s) => s.automationOpen);
  const request = useAppStore((s) => s.automationRequest);
  const context = useAppStore((s) => s.automationContext);
  const setAutomationOpen = useAppStore((s) => s.setAutomationOpen);
  const setAutomationState = useAppStore((s) => s.setAutomationState);
  const clearRequest = useAppStore((s) => s.clearAutomationRequest);
  const setAutomationContext = useAppStore((s) => s.setAutomationContext);
  const setSubmitResult = useAppStore((s) => s.setSubmitResult);
  const hostRef = useRef<HTMLDivElement | null>(null);
  const openedKey = useRef("");
  const platform = automation?.platform ?? request?.platform ?? "";
  const visible = automationOpen || automation?.needsHuman === true;

  const reportBounds = useCallback(() => {
    if (!platform || !hostRef.current) return;
    const rect = hostRef.current.getBoundingClientRect();
    if (rect.width < 60 || rect.height < 60) return;
    api
      .setAutomationBounds(platform, {
        x: rect.left,
        y: rect.top,
        width: rect.width,
        height: rect.height,
        viewportHeight: window.innerHeight,
      })
      .catch(() => {});
  }, [platform]);

  useEffect(() => {
    if (!visible) return;
    const handle = window.setTimeout(reportBounds, 30);
    window.addEventListener("resize", reportBounds);
    const observer = new ResizeObserver(reportBounds);
    if (hostRef.current) observer.observe(hostRef.current);
    const interval = window.setInterval(reportBounds, 1200);
    return () => {
      window.clearTimeout(handle);
      window.removeEventListener("resize", reportBounds);
      observer.disconnect();
      window.clearInterval(interval);
    };
  }, [visible, reportBounds]);

  useEffect(() => {
    if (!visible || !request) return;
    const key = `${request.platform}|${request.url}`;
    if (openedKey.current === key) return;
    openedKey.current = key;
    const timer = window.setTimeout(async () => {
      const rect = hostRef.current?.getBoundingClientRect();
      if (!rect) return;
      try {
        await api.openAutomation(request.platform, request.url, {
          x: rect.left,
          y: rect.top,
          width: rect.width,
          height: rect.height,
          viewportHeight: window.innerHeight,
        });
        clearRequest();
      } catch (error) {
        message.error(errText(error));
        clearRequest();
      }
    }, 80);
    return () => window.clearTimeout(timer);
  }, [visible, request, clearRequest, message]);

  const close = async () => {
    try {
      if (platform) await api.closeAutomation(platform);
    } catch {
      // ignore
    }
    openedKey.current = "";
    setAutomationOpen(false);
    setAutomationState(null);
    clearRequest();
    setAutomationContext(null);
  };

  const verify = async () => {
    if (!platform) return;
    try {
      const result = await api.verifyPlatformLogin(platform);
      if (result.loggedIn) {
        message.success(result.message);
        setAutomationOpen(false);
      } else {
        message.warning(result.message);
      }
    } catch (error) {
      message.error(errText(error));
    }
  };

  const capture = async () => {
    if (!platform || !context) return;
    try {
      message.loading({ content: "正在读取判定…", key: "capture" });
      const result = await api.captureVerdict({
        problemId: context.problemId,
        code: context.code,
        language: context.language,
        sinceEpoch: context.since,
      });
      message.success({ content: result.message, key: "capture" });
      setSubmitResult(result);
      setAutomationOpen(false);
    } catch (error) {
      message.error({ content: errText(error), key: "capture" });
    }
  };

  if (!visible || !platform) return null;

  return (
    <div className="automation-overlay">
      <div className="automation-panel">
        <div className="automation-header">
          <strong>{platformName(platform)} 自动化面板</strong>
          <Tag color={statusColor(automation?.status ?? "running")}>
            {automationStatusLabel(automation?.status)}
          </Tag>
          <span className="muted tiny grow">
            {automation?.message || "正在准备页面…"}
          </span>
          <Button
            type="text"
            size="small"
            icon={<CloseOutlined />}
            onClick={close}
          >
            关闭
          </Button>
        </div>
        <div ref={hostRef} className="automation-host">
          {automation?.needsHuman ? "等待你在此页面完成操作" : "内置浏览器页面加载中…"}
        </div>
        <div className="automation-footer">
          {automation?.needsHuman ? (
            <Alert
              type="warning"
              showIcon
              style={{ width: "100%" }}
              title={automation.message || "需要人工处理"}
              description={
                automation.hint ||
                "请在右侧页面完成登录/验证码/确认提交，然后回到这里继续。"
              }
            />
          ) : (
            <span className="muted tiny">
              页面与登录态保存在应用内；不会自动重试可能产生重复提交的操作。
            </span>
          )}
          <Space wrap>
            <Button size="small" icon={<ReloadOutlined />} onClick={verify}>
              验证登录 / 继续
            </Button>
            {context ? (
              <Button size="small" type="primary" onClick={capture}>
                我已提交，读取结果
              </Button>
            ) : null}
          </Space>
        </div>
      </div>
    </div>
  );
}
