import { Layout, Menu, Tag } from "antd";
import {
  BarChartOutlined,
  BookOutlined,
  FileTextOutlined,
  MessageOutlined,
  SettingOutlined,
} from "@ant-design/icons";
import { Outlet, useLocation, useNavigate } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import { api } from "../lib/api";

const { Sider, Content } = Layout;

export default function Shell() {
  const navigate = useNavigate();
  const location = useLocation();
  const appInfo = useQuery({ queryKey: ["app-info"], queryFn: api.getAppInfo });
  const roleModels = useQuery({ queryKey: ["role-models"], queryFn: api.getRoleModels });
  const chatRole = roleModels.data?.find((role) => role.role === "chat");
  const selected =
    location.pathname === "/"
      ? "/"
      : "/" + location.pathname.split("/").filter(Boolean)[0];

  return (
    <Layout className="app-shell">
      <Sider width={232} className="app-sider" theme="dark">
        <div className="brand">
          <div className="brand-mark">IC</div>
          <div>
            <div className="brand-title">ICPC 备赛工作站</div>
            <div className="brand-sub">training station</div>
          </div>
        </div>
        <Menu
          theme="dark"
          mode="inline"
          selectedKeys={[selected]}
          onClick={({ key }) => navigate(key)}
          items={[
            { key: "/", icon: <MessageOutlined />, label: "Agent" },
            { key: "/dashboard", icon: <BarChartOutlined />, label: "数据面板" },
            { key: "/training", icon: <BookOutlined />, label: "训练" },
            { key: "/reports", icon: <FileTextOutlined />, label: "报告" },
            { key: "/settings", icon: <SettingOutlined />, label: "设置" },
          ]}
          style={{ background: "transparent", borderInlineEnd: "none" }}
        />
        <div className="sider-footer">
          <div className="flex between center gap-8">
            <span>模型</span>
            {chatRole?.modelId ? (
              <Tag color="cyan" style={{ maxWidth: 130, overflow: "hidden" }}>
                {chatRole.modelId}
              </Tag>
            ) : (
              <Tag color="orange">未配置</Tag>
            )}
          </div>
          <div className="flex between center gap-8">
            <span>版本</span>
            <span className="mono">{appInfo.data?.version ?? "0.1.0"}</span>
          </div>
        </div>
      </Sider>
      <Content style={{ height: "100vh", overflow: "hidden" }}>
        <Outlet />
      </Content>
    </Layout>
  );
}
