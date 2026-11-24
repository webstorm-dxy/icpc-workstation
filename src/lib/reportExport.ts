import { unified } from "unified";
import remarkParse from "remark-parse";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";
import rehypeKatex from "rehype-katex";
import rehypeSanitize from "rehype-sanitize";
import rehypeStringify from "rehype-stringify";
import type { Report } from "../types";

function escapeHtml(input: string): string {
  return input
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

type ReportData = {
  period?: { start?: string; end?: string };
  stats?: { submissions?: number; accepted?: number; acceptance_rate?: number };
  accounts?: Array<{ platform: string; rating?: number; solved?: number }>;
  skill_tags?: Array<{ tag: string; rating: number; attempts: number; solved: number }>;
  submissions?: Array<{ title: string; platform: string; verdict: string; rating?: number }>;
  analyses?: Array<{
    title: string;
    verdict: string;
    summary?: string;
    category?: string;
    root_cause?: string;
  }>;
  active_plan?: { total?: number; done?: number; rationale?: string };
};

function tagBars(tags: NonNullable<ReportData["skill_tags"]>): string {
  if (!tags.length) return "<p class='muted'>暂无标签数据</p>";
  const max = Math.max(2400, ...tags.map((tag) => tag.rating));
  return tags
    .slice(0, 12)
    .map((tag) => {
      const width = Math.max(2, Math.round((tag.rating / max) * 100));
      const color = tag.rating < 1200 ? "#ef4444" : tag.rating < 1400 ? "#f59e0b" : "#22c55e";
      return `<div style="display:flex;align-items:center;gap:10px;margin:6px 0">
        <span style="width:130px;font-size:12px;color:#555">${escapeHtml(tag.tag)}</span>
        <div style="flex:1;background:#eee;height:10px;border-radius:5px"><div style="width:${width}%;height:10px;border-radius:5px;background:${color}"></div></div>
        <span style="width:56px;text-align:right;font-size:12px;color:#333">${Math.round(tag.rating)}</span>
      </div>`;
    })
    .join("");
}

export async function buildReportHtml(report: Report): Promise<string> {
  const data = (report.data ?? {}) as ReportData;
  let bodyHtml = "";
  try {
    const file = await unified()
      .use(remarkParse)
      .use(remarkGfm)
      .use(remarkMath)
      .use(rehypeSanitize)
      .use(rehypeKatex, { output: "mathml" })
      .use(rehypeStringify)
      .process(report.contentMd);
    bodyHtml = String(file);
  } catch {
    bodyHtml = `<pre>${escapeHtml(report.contentMd)}</pre>`;
  }
  const stats = data.stats ?? {};
  const submissions = (data.submissions ?? [])
    .slice(0, 30)
    .map(
      (row) =>
        `<tr><td>${escapeHtml(row.title)}</td><td>${escapeHtml(row.platform)}</td><td>${escapeHtml(row.verdict)}</td><td>${row.rating ?? "—"}</td></tr>`,
    )
    .join("");
  const analyses = (data.analyses ?? [])
    .slice(0, 8)
    .map(
      (row) =>
        `<div class="analysis"><strong>${escapeHtml(row.title)} · ${escapeHtml(row.verdict)}</strong><p>${escapeHtml(row.summary ?? "")}</p><p class="muted">${escapeHtml(row.root_cause ?? "")}</p></div>`,
    )
    .join("");
  const period = data.period ?? {};
  return `<!doctype html>
<html lang="zh-CN"><head><meta charset="utf-8">
<title>${escapeHtml(report.title)}</title>
<style>
  body{font-family:-apple-system,BlinkMacSystemFont,"Segoe UI","PingFang SC",sans-serif;max-width:920px;margin:36px auto;padding:0 32px;color:#111;line-height:1.7}
  h1{font-size:28px;margin:0 0 6px} h2{font-size:20px;margin:26px 0 10px;border-bottom:1px solid #eee;padding-bottom:6px} h3{font-size:16px}
  .muted{color:#777} .meta{color:#777;font-size:13px;margin-bottom:22px}
  .stats{display:grid;grid-template-columns:repeat(3,1fr);gap:12px;margin:18px 0}
  .stat{border:1px solid #e5e7eb;border-radius:10px;padding:12px 14px}
  .stat .num{font-size:24px;font-weight:700} .stat .label{font-size:12px;color:#777}
  table{width:100%;border-collapse:collapse;font-size:13px;margin:12px 0}
  th,td{border-bottom:1px solid #eee;padding:7px 8px;text-align:left} th{background:#fafafa}
  pre{background:#f6f6f6;padding:12px;border-radius:8px;overflow:auto;font-size:12px}
  code{background:#f3f4f6;padding:1px 4px;border-radius:4px;font-size:.9em}
  .analysis{border-left:3px solid #e5e7eb;padding:6px 12px;margin:10px 0;background:#fafafa}
  .analysis p{margin:3px 0;font-size:13px}
  ul,ol{padding-left:22px}
  @media print{body{margin:0;max-width:none}}
</style></head>
<body>
<h1>${escapeHtml(report.title)}</h1>
<div class="meta">统计区间：${escapeHtml(period.start ?? "—")} 至 ${escapeHtml(period.end ?? "—")} · 生成时间：${escapeHtml(report.createdAt)}</div>
<div class="stats">
  <div class="stat"><div class="num">${stats.submissions ?? 0}</div><div class="label">提交数</div></div>
  <div class="stat"><div class="num">${stats.accepted ?? 0}</div><div class="label">通过数</div></div>
  <div class="stat"><div class="num">${(((stats.acceptance_rate ?? 0) as number) * 100).toFixed(1)}%</div><div class="label">通过率</div></div>
</div>
<h2>标签能力</h2>
${tagBars(data.skill_tags ?? [])}
<h2>训练报告</h2>
${bodyHtml}
${submissions ? `<h2>提交明细</h2><table><thead><tr><th>题目</th><th>平台</th><th>判定</th><th>难度</th></tr></thead><tbody>${submissions}</tbody></table>` : ""}
${analyses ? `<h2>代码分析摘要</h2>${analyses}` : ""}
<p class="muted" style="margin-top:32px;font-size:12px">由 ICPC 备赛工作站生成 · 数据仅保存在本机</p>
</body></html>`;
}
