import {
  Alert,
  App,
  Avatar,
  Button,
  Collapse,
  Empty,
  Input,
  Popconfirm,
  Space,
  Spin,
  Tag,
  Tooltip,
  Typography,
} from "antd";
import {
  DeleteOutlined,
  PlusOutlined,
  SendOutlined,
  StopOutlined,
  ThunderboltOutlined,
  UserOutlined,
} from "@ant-design/icons";
import { useEffect, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "react-router-dom";
import { api, errText, events } from "../lib/api";
import { formatDate, statusColor } from "../lib/format";
import { MarkdownView } from "../lib/markdown";
import { useAppStore } from "../store";
import type { ChatMessage, Conversation, ToolCall } from "../types";

type LiveMessage = {
  id: string;
  conversationId: string;
  content: string;
  reasoning: string;
  tools: ToolCall[];
};

const PROMPTS = [
  {
    title: "同步并总结我的训练状态",
    desc: "读取 Codeforces / AtCoder / 洛谷 的 rating、提交与薄弱标签。",
    prompt: "同步我的 OJ 数据，然后用一段话总结我最近的训练状态和最需要补的短板。",
  },
  {
    title: "生成今天的训练计划",
    desc: "结合最近提交与薄弱标签，推荐具体题目。",
    prompt: "根据我的训练数据，生成一份今天的训练计划，说明每道题为什么推荐我。",
  },
  {
    title: "复盘最近一次失败",
    desc: "从代码、样例与判定中定位问题。",
    prompt: "分析我最近一次没有通过的提交，指出最可能的根因和最小修复步骤。",
  },
  {
    title: "制定下一阶段路线",
    desc: "rating、标签与时间预算一起考虑。",
    prompt: "我现在应该优先补哪些标签？给我一个两周可执行的训练路线。",
  },
];

function ToolCards({ tools }: { tools: ToolCall[] }) {
  if (!tools.length) return null;
  return (
    <div style={{ marginBottom: 8 }}>
      {tools.map((tool) => (
        <div className="tool-card" key={tool.id}>
          <span className="tool-name">{tool.name}</span>
          <Tag color={statusColor(tool.status)} style={{ marginInlineEnd: 0 }}>
            {tool.status}
          </Tag>
          <span className="muted">{tool.summary}</span>
        </div>
      ))}
    </div>
  );
}

function MessageRow({
  message,
  streaming = false,
  reasoning,
}: {
  message: ChatMessage;
  streaming?: boolean;
  reasoning?: string;
}) {
  const isUser = message.role === "user";
  return (
    <div className={`msg-row ${isUser ? "user" : ""}`}>
      <div className={`msg-avatar ${isUser ? "user" : "agent"}`}>
        {isUser ? <UserOutlined /> : "IC"}
      </div>
      <div className="msg-bubble">
        {isUser ? (
          <div style={{ whiteSpace: "pre-wrap" }}>{message.content}</div>
        ) : (
          <>
            <ToolCards tools={message.toolCalls} />
            {reasoning ? (
              <Collapse
                ghost
                size="small"
                items={[
                  {
                    key: "reasoning",
                    label: <span className="muted tiny">思考过程</span>,
                    children: (
                      <div className="muted tiny" style={{ whiteSpace: "pre-wrap" }}>
                        {reasoning}
                      </div>
                    ),
                  },
                ]}
                style={{ marginBottom: 6 }}
              />
            ) : null}
            {message.content ? <MarkdownView content={message.content} /> : null}
            {streaming ? <span className="cursor-blink" /> : null}
            <div className="msg-meta">
              {formatDate(message.createdAt)}
              {message.usage?.total_tokens ? (
                <span> · {String(message.usage.total_tokens)} tokens</span>
              ) : null}
            </div>
          </>
        )}
      </div>
    </div>
  );
}

export default function ChatPage() {
  const { message } = App.useApp();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const onboarding = useAppStore((s) => s.onboarding);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [live, setLive] = useState<LiveMessage | null>(null);
  const liveRef = useRef<LiveMessage | null>(null);
  const [sending, setSending] = useState(false);
  const [input, setInput] = useState("");
  const deltaBuffer = useRef(new Map<string, string>());
  const reasoningBuffer = useRef(new Map<string, string>());
  const toolBuffer = useRef(new Map<string, ToolCall[]>());

  useEffect(() => {
    liveRef.current = live;
  }, [live]);

  const conversations = useQuery({
    queryKey: ["conversations"],
    queryFn: api.listConversations,
  });
  const messages = useQuery({
    queryKey: ["messages", activeId],
    queryFn: () => (activeId ? api.getMessages(activeId) : Promise.resolve([] as ChatMessage[])),
    enabled: true,
  });
  const providers = useQuery({ queryKey: ["providers"], queryFn: api.listProviders });

  useEffect(() => {
    if (!activeId && conversations.data?.length) {
      setActiveId(conversations.data[0].id);
    }
  }, [activeId, conversations.data]);

  useEffect(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];
    const register = async () => {
      const list = await Promise.all([
        events.onAgentDelta((event) => {
          if (liveRef.current?.id === event.messageId) {
            setLive((current) =>
              current && current.id === event.messageId
                ? { ...current, content: current.content + event.delta }
                : current,
            );
          } else {
            const prev = deltaBuffer.current.get(event.messageId) ?? "";
            deltaBuffer.current.set(event.messageId, prev + event.delta);
          }
        }),
        events.onAgentReasoning((event) => {
          if (liveRef.current?.id === event.messageId) {
            setLive((current) =>
              current && current.id === event.messageId
                ? { ...current, reasoning: current.reasoning + event.delta }
                : current,
            );
          } else {
            const prev = reasoningBuffer.current.get(event.messageId) ?? "";
            reasoningBuffer.current.set(event.messageId, prev + event.delta);
          }
        }),
        events.onAgentTool((event) => {
          if (liveRef.current?.id === event.messageId) {
            setLive((current) => {
              if (!current || current.id !== event.messageId) return current;
              const tools = [...current.tools];
              const index = tools.findIndex((tool) => tool.id === event.tool.id);
              if (index >= 0) tools[index] = event.tool;
              else tools.push(event.tool);
              return { ...current, tools };
            });
          } else {
            const tools = toolBuffer.current.get(event.messageId) ?? [];
            const index = tools.findIndex((tool) => tool.id === event.tool.id);
            if (index >= 0) tools[index] = event.tool;
            else tools.push(event.tool);
            toolBuffer.current.set(event.messageId, tools);
          }
        }),
        events.onAgentDone((event) => {
          if (liveRef.current?.id === event.messageId) {
            setLive(null);
            setSending(false);
          }
          deltaBuffer.current.delete(event.messageId);
          reasoningBuffer.current.delete(event.messageId);
          toolBuffer.current.delete(event.messageId);
          if (event.error) message.error(event.error);
          queryClient.invalidateQueries({ queryKey: ["conversations"] });
          queryClient.invalidateQueries({ queryKey: ["messages"] });
          queryClient.invalidateQueries({ queryKey: ["dashboard"] });
          queryClient.invalidateQueries({ queryKey: ["plan"] });
        }),
      ]);
      if (disposed) {
        list.forEach((fn) => fn());
        return;
      }
      list.forEach((fn) => unlisteners.push(fn));
    };
    register().catch(() => {});
    return () => {
      disposed = true;
      unlisteners.forEach((fn) => fn());
    };
  }, [message, queryClient]);

  const createConversation = async (): Promise<string | null> => {
    try {
      const conversation = await api.createConversation();
      await queryClient.invalidateQueries({ queryKey: ["conversations"] });
      setActiveId(conversation.id);
      setLive(null);
      return conversation.id;
    } catch (error) {
      message.error(errText(error));
      return null;
    }
  };

  const send = async (text?: string) => {
    const content = (text ?? input).trim();
    if (!content || sending) return;
    if (!providers.data?.length) {
      message.warning("请先配置模型提供商");
      navigate("/settings");
      return;
    }
    let conversationId = activeId;
    if (!conversationId) {
      conversationId = await createConversation();
      if (!conversationId) return;
    }
    setInput("");
    setSending(true);
    try {
      const result = await api.sendMessage(conversationId, content);
      const bufferedContent = deltaBuffer.current.get(result.assistantMessageId) ?? "";
      const bufferedReasoning = reasoningBuffer.current.get(result.assistantMessageId) ?? "";
      const bufferedTools = toolBuffer.current.get(result.assistantMessageId) ?? [];
      deltaBuffer.current.delete(result.assistantMessageId);
      reasoningBuffer.current.delete(result.assistantMessageId);
      toolBuffer.current.delete(result.assistantMessageId);
      setLive({
        id: result.assistantMessageId,
        conversationId,
        content: bufferedContent,
        reasoning: bufferedReasoning,
        tools: bufferedTools,
      });
      setActiveId(conversationId);
      await queryClient.invalidateQueries({ queryKey: ["messages", conversationId] });
    } catch (error) {
      setSending(false);
      message.error(errText(error));
    }
  };

  const stop = async () => {
    if (!live) return;
    try {
      await api.cancelMessage(live.id);
    } catch (error) {
      message.error(errText(error));
    }
  };

  const removeConversation = async (conversation: Conversation) => {
    try {
      await api.deleteConversation(conversation.id);
      if (activeId === conversation.id) {
        setActiveId(null);
        setLive(null);
      }
      await queryClient.invalidateQueries({ queryKey: ["conversations"] });
    } catch (error) {
      message.error(errText(error));
    }
  };

  const selectConversation = (id: string) => {
    setActiveId(id);
    if (live && live.conversationId !== id) {
      // The stream continues in the backend; this conversation simply hides it.
    }
  };

  const history = messages.data ?? [];
  const liveForActive = live && live.conversationId === activeId ? live : null;
  const showWelcome = !activeId && history.length === 0 && !liveForActive;

  return (
    <div className="chat-layout">
      <aside className="chat-sidebar">
        <div style={{ padding: 14, borderBottom: "1px solid var(--border)" }}>
          <Button
            block
            icon={<PlusOutlined />}
            onClick={() => createConversation()}
          >
            新建对话
          </Button>
        </div>
        <div style={{ flex: 1, overflowY: "auto", padding: 8 }}>
          {conversations.isLoading ? (
            <div style={{ padding: 20, textAlign: "center" }}>
              <Spin size="small" />
            </div>
          ) : conversations.data?.length ? (
            conversations.data.map((conversation) => (
              <div
                key={conversation.id}
                onClick={() => selectConversation(conversation.id)}
                style={{
                  padding: "9px 10px",
                  borderRadius: 8,
                  cursor: "pointer",
                  background:
                    activeId === conversation.id
                      ? "rgba(34, 211, 238, 0.10)"
                      : "transparent",
                  border:
                    activeId === conversation.id
                      ? "1px solid rgba(34, 211, 238, 0.25)"
                      : "1px solid transparent",
                  marginBottom: 4,
                }}
              >
                <div className="flex between center gap-8">
                  <div
                    style={{
                      overflow: "hidden",
                      textOverflow: "ellipsis",
                      whiteSpace: "nowrap",
                      fontSize: 13,
                    }}
                  >
                    {conversation.title}
                  </div>
                  <Popconfirm
                    title="删除这个对话？"
                    onConfirm={(event) => {
                      event?.stopPropagation();
                      removeConversation(conversation);
                    }}
                    onCancel={(event) => event?.stopPropagation()}
                  >
                    <Button
                      type="text"
                      size="small"
                      icon={<DeleteOutlined />}
                      onClick={(event) => event.stopPropagation()}
                    />
                  </Popconfirm>
                </div>
                <div className="muted tiny">{formatDate(conversation.updatedAt, false)}</div>
              </div>
            ))
          ) : (
            <div className="muted tiny" style={{ padding: 12 }}>
              还没有对话。点上方按钮开始。
            </div>
          )}
        </div>
        <div style={{ padding: 12, borderTop: "1px solid var(--border)" }}>
          <div className="muted tiny">
            {onboarding?.configuredPlatforms?.length
              ? `已接入：${onboarding.configuredPlatforms.join(" / ")}`
              : "尚未接入 OJ，可在设置中配置"}
          </div>
        </div>
      </aside>

      <main className="chat-main">
        {providers.data && providers.data.length === 0 ? (
          <div style={{ padding: "12px 24px 0" }}>
            <Alert
              type="warning"
              showIcon
              title="还没有配置模型提供商"
              description="配置 OpenCode Go 或 DeepSeek 后，Agent 才能生成计划、分析代码和写报告。"
              action={
                <Button size="small" type="primary" onClick={() => navigate("/settings")}>
                  去配置
                </Button>
              }
            />
          </div>
        ) : null}
        <div className="chat-scroll">
          {showWelcome ? (
            <div className="welcome">
              <Tag color="cyan" icon={<ThunderboltOutlined />}>
                ICPC AGENT
              </Tag>
              <h1>
                你的 <span className="gradient-text">ICPC 训练搭档</span>
                <br />
                从数据到下一步行动
              </h1>
              <p className="muted" style={{ fontSize: 14, maxWidth: 640 }}>
                我可以同步你的 OJ 数据、解释薄弱标签、生成训练计划、陪你复盘代码。
                需要写数据或操作浏览器时，我会先请你确认。
              </p>
              <div className="prompt-grid">
                {PROMPTS.map((prompt) => (
                  <div
                    key={prompt.title}
                    className="prompt-card"
                    onClick={() => send(prompt.prompt)}
                  >
                    <div className="title">{prompt.title}</div>
                    <div className="desc">{prompt.desc}</div>
                  </div>
                ))}
              </div>
            </div>
          ) : null}

          {!showWelcome && messages.isLoading ? (
            <div style={{ textAlign: "center", padding: 40 }}>
              <Spin />
            </div>
          ) : null}

          {history.map((item) => (
            <MessageRow key={item.id} message={item} />
          ))}

          {liveForActive ? (
            <MessageRow
              streaming
              reasoning={liveForActive.reasoning}
              message={{
                id: liveForActive.id,
                role: "assistant",
                content: liveForActive.content,
                createdAt: new Date().toISOString(),
                toolCalls: liveForActive.tools,
                usage: null,
              }}
            />
          ) : null}

          {!showWelcome && !history.length && !liveForActive && !messages.isLoading ? (
            <Empty description="这个对话还没有消息" />
          ) : null}
        </div>

        <div className="chat-composer-wrap">
          <Input.TextArea
            value={input}
            onChange={(event) => setInput(event.target.value)}
            onPressEnter={(event) => {
              if (!event.shiftKey) {
                event.preventDefault();
                send();
              }
            }}
            autoSize={{ minRows: 1, maxRows: 6 }}
            placeholder="问训练数据、让 Agent 生成计划，或贴一段代码让它复盘…"
            style={{ fontSize: 14, padding: "10px 12px" }}
            disabled={sending}
          />
          <div className="flex between center mt-8">
            <span className="muted tiny">Enter 发送 · Shift+Enter 换行</span>
            <Space>
              {sending && live ? (
                <Tooltip title="停止生成">
                  <Button icon={<StopOutlined />} onClick={stop}>
                    停止
                  </Button>
                </Tooltip>
              ) : null}
              <Button
                type="primary"
                icon={<SendOutlined />}
                loading={sending}
                disabled={!input.trim()}
                onClick={() => send()}
              >
                发送
              </Button>
            </Space>
          </div>
        </div>
      </main>
    </div>
  );
}
