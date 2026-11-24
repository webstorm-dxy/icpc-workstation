import dayjs from "dayjs";

export const PLATFORM_NAMES: Record<string, string> = {
  codeforces: "Codeforces",
  atcoder: "AtCoder",
  luogu: "洛谷",
};

export function platformName(platform: string): string {
  return PLATFORM_NAMES[platform] ?? platform;
}

export function verdictColor(verdict: string): string {
  switch (verdict) {
    case "AC":
      return "green";
    case "WA":
      return "red";
    case "TLE":
      return "orange";
    case "MLE":
      return "volcano";
    case "RE":
      return "magenta";
    case "CE":
      return "purple";
    case "JUDGING":
      return "blue";
    default:
      return "default";
  }
}

export function verdictLabel(verdict: string): string {
  const map: Record<string, string> = {
    AC: "通过",
    WA: "答案错误",
    TLE: "超时",
    MLE: "内存超限",
    RE: "运行错误",
    CE: "编译错误",
    PE: "格式错误",
    OLE: "输出超限",
    ILE: "空闲超限",
    JUDGING: "评测中",
    PARTIAL: "部分分",
    FAILED: "失败",
    SKIPPED: "跳过",
    UNKNOWN: "未知",
  };
  return map[verdict] ?? verdict;
}

export function statusColor(status: string): string {
  switch (status) {
    case "done":
    case "OK":
    case "AC":
      return "green";
    case "running":
    case "judging":
    case "opening":
      return "blue";
    case "ready":
      return "green";
    case "error":
    case "WA":
    case "RE":
      return "red";
    case "needs_human":
      return "orange";
    default:
      return "default";
  }
}

export function automationStatusLabel(status?: string | null): string {
  switch (status) {
    case "opening":
      return "打开中";
    case "ready":
      return "已就绪";
    case "running":
      return "进行中";
    case "needs_human":
      return "需要你操作";
    case "idle":
      return "已关闭";
    case "error":
      return "出错";
    default:
      return status || "进行中";
  }
}

export function formatDate(value?: string | null, withTime = true): string {
  if (!value) return "—";
  const d = dayjs(value);
  if (!d.isValid()) return value;
  return d.format(withTime ? "YYYY-MM-DD HH:mm" : "YYYY-MM-DD");
}

export function fromNow(value?: string | null): string {
  if (!value) return "—";
  return dayjs(value).fromNow?.() ?? formatDate(value);
}

export function difficultyLabel(rating?: number | null): string {
  if (!rating) return "未定级";
  if (rating < 1000) return "入门";
  if (rating < 1400) return "普及";
  if (rating < 1700) return "提高-";
  if (rating < 1900) return "提高+";
  if (rating < 2100) return "省选-";
  if (rating < 2400) return "省选+";
  if (rating < 2700) return "NOI";
  return "集训队";
}

export function planKindLabel(kind: string): string {
  switch (kind) {
    case "review":
      return "复盘";
    case "stretch":
      return "拉伸";
    default:
      return "练习";
  }
}

export function failureCategoryLabel(category: string): string {
  const map: Record<string, string> = {
    algorithm: "算法选择",
    implementation: "实现细节",
    complexity: "复杂度",
    edge_case: "边界条件",
    io: "输入输出",
    overflow: "溢出",
    recursion: "递归",
    unknown: "待定位",
    none: "无失败",
  };
  return map[category] ?? category;
}

export function roleLabel(role: string): string {
  const map: Record<string, string> = {
    chat: "对话",
    recommend: "推荐",
    analyze: "分析",
    report: "报告",
  };
  return map[role] ?? role;
}

export function shortPath(path?: string | null): string {
  if (!path) return "—";
  const parts = path.split("/");
  return parts.slice(-2).join("/");
}
