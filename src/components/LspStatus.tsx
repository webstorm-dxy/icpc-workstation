import { ApiOutlined, ReloadOutlined } from "@ant-design/icons";
import { Dropdown, Tag, Tooltip, type MenuProps } from "antd";
import type { ItemType } from "antd/es/menu/interface";
import { useMemo } from "react";
import type { LspServer } from "../types";
import type { LspStatus } from "../lib/lsp";

const PHASE_LABEL: Record<LspStatus["phase"], string> = {
  disabled: "LSP 已关闭",
  starting: "clangd 启动中",
  ready: "clangd 就绪",
  error: "LSP 不可用",
};

const PHASE_COLOR: Record<LspStatus["phase"], string> = {
  disabled: "default",
  starting: "processing",
  ready: "green",
  error: "red",
};

function baseName(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] || path;
}

export interface LspStatusTagProps {
  status: LspStatus;
  servers: LspServer[];
  serverPath?: string | null;
  capabilities: string[];
  onSelectServer: (path: string | null) => void;
  onRestart: () => void;
  onRefreshServers: () => void;
  onToggle: () => void;
}

/**
 * Toolbar indicator for the language server.
 *
 * Everything the user can do about clangd lives behind this tag: pick a
 * detected binary, restart the session, refresh detection or turn LSP off.
 */
export function LspStatusTag({
  status,
  servers,
  serverPath,
  capabilities,
  onSelectServer,
  onRestart,
  onRefreshServers,
  onToggle,
}: LspStatusTagProps) {
  const items = useMemo<MenuProps["items"]>(() => {
    const detected: ItemType[] = servers.length
      ? servers.map((server) => ({
          key: `server:${server.path}`,
          label: (
            <div style={{ display: "flex", flexDirection: "column", lineHeight: 1.3 }}>
              <span>{baseName(server.path)}</span>
              <span className="muted tiny">{server.version ?? server.path}</span>
            </div>
          ),
        }))
      : [{ key: "server:none", label: "未检测到 clangd", disabled: true }];

    return [
      { key: "group-servers", type: "group", label: "语言服务器", children: detected },
      {
        key: "server:auto",
        label: "自动选择（PATH 中最近的 clangd）",
        disabled: !servers.length,
      },
      {
        key: "refresh",
        label: "重新检测服务器",
        icon: <ReloadOutlined />,
      },
      { type: "divider" },
      { key: "restart", label: "重启语言服务器", disabled: status.phase === "disabled" },
      {
        key: "toggle",
        label: status.phase === "disabled" ? "开启 LSP" : "关闭 LSP",
      },
    ];
  }, [servers, status.phase]);

  const onClick: MenuProps["onClick"] = ({ key }) => {
    if (key === "refresh") {
      onRefreshServers();
      return;
    }
    if (key === "restart") {
      onRestart();
      return;
    }
    if (key === "toggle") {
      onToggle();
      return;
    }
    if (key === "server:auto") {
      onSelectServer(null);
      return;
    }
    if (key.startsWith("server:")) {
      onSelectServer(key.slice("server:".length));
    }
  };

  const tooltip = (
    <div style={{ maxWidth: 320, lineHeight: 1.6 }}>
      <div>
        {status.phase === "disabled"
          ? "代码编辑器当前只提供本地补全。开启后会启动 clangd，支持语义补全、悬停、诊断、跳转与格式化。"
          : status.server ?? "尚未选择语言服务器"}
      </div>
      {status.serverVersion ? <div className="muted tiny">{status.serverVersion}</div> : null}
      {capabilities.length ? <div>能力：{capabilities.join(" · ")}</div> : null}
      {status.error ? <div style={{ color: "#f87171" }}>{status.error}</div> : null}
      <div className="muted tiny">点击可切换服务器、重启或关闭</div>
    </div>
  );

  return (
    <Dropdown
      menu={{
        items,
        onClick,
        selectable: true,
        selectedKeys: [serverPath ? `server:${serverPath}` : "server:auto"],
      }}
      trigger={["click"]}
      placement="bottomLeft"
    >
      <Tooltip title={tooltip} placement="bottom">
        <Tag
          color={PHASE_COLOR[status.phase]}
          icon={<ApiOutlined />}
          style={{ cursor: "pointer", marginInlineEnd: 0 }}
        >
          {PHASE_LABEL[status.phase]}
          {status.phase === "ready" && serverPath ? ` · ${baseName(serverPath)}` : ""}
        </Tag>
      </Tooltip>
    </Dropdown>
  );
}
