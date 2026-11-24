import * as echarts from "echarts";
import { useEffect, useRef } from "react";

export function EChart({
  option,
  height = 280,
  style,
}: {
  option: Record<string, unknown>;
  height?: number | string;
  style?: React.CSSProperties;
}) {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const chartRef = useRef<echarts.ECharts | null>(null);

  useEffect(() => {
    if (!hostRef.current) return;
    const chart = echarts.init(hostRef.current, undefined, {
      renderer: "canvas",
    });
    chartRef.current = chart;
    const observer = new ResizeObserver(() => chart.resize());
    observer.observe(hostRef.current);
    return () => {
      observer.disconnect();
      chart.dispose();
      chartRef.current = null;
    };
  }, []);

  useEffect(() => {
    chartRef.current?.setOption(
      option as echarts.EChartsOption,
      true,
    );
  }, [option]);

  return <div ref={hostRef} style={{ width: "100%", height, ...style }} />;
}

export function ratingChartOption(
  points: { at: string; rating: number; platform: string }[],
): Record<string, unknown> {
  const platforms = Array.from(new Set(points.map((p) => p.platform)));
  const colors = ["#22d3ee", "#a78bfa", "#fbbf24", "#34d399"];
  return {
    backgroundColor: "transparent",
    grid: { left: 44, right: 20, top: 30, bottom: 32 },
    tooltip: { trigger: "axis" },
    legend: {
      top: 0,
      textStyle: { color: "#8b9bb0" },
    },
    xAxis: {
      type: "category",
      data: Array.from(new Set(points.map((p) => p.at.slice(0, 10)))),
      axisLine: { lineStyle: { color: "#223041" } },
      axisLabel: { color: "#8b9bb0", fontSize: 11 },
    },
    yAxis: {
      type: "value",
      scale: true,
      axisLabel: { color: "#8b9bb0", fontSize: 11 },
      splitLine: { lineStyle: { color: "#182230" } },
    },
    series: platforms.map((platform, index) => ({
      name: platform,
      type: "line",
      smooth: true,
      symbolSize: 6,
      showSymbol: points.filter((p) => p.platform === platform).length < 30,
      data: points
        .filter((p) => p.platform === platform)
        .map((p) => [p.at.slice(0, 10), p.rating]),
      lineStyle: { width: 2 },
      itemStyle: { color: colors[index % colors.length] },
      areaStyle: {
        color: new echarts.graphic.LinearGradient(0, 0, 0, 1, [
          { offset: 0, color: `${colors[index % colors.length]}44` },
          { offset: 1, color: "transparent" },
        ]),
      },
    })),
  };
}

export function tagBarOption(
  tags: { tag: string; rating: number; attempts: number }[],
): Record<string, unknown> {
  const data = tags.slice(0, 10).reverse();
  return {
    backgroundColor: "transparent",
    grid: { left: 100, right: 24, top: 8, bottom: 18 },
    tooltip: { trigger: "axis" },
    xAxis: {
      type: "value",
      min: 800,
      max: 2400,
      axisLabel: { color: "#8b9bb0", fontSize: 11 },
      splitLine: { lineStyle: { color: "#182230" } },
    },
    yAxis: {
      type: "category",
      data: data.map((t) => t.tag),
      axisLabel: { color: "#c7d2dc", fontSize: 11 },
      axisLine: { lineStyle: { color: "#223041" } },
    },
    series: [
      {
        type: "bar",
        data: data.map((t) => ({
          value: Math.round(t.rating),
          itemStyle: {
            color:
              t.rating < 1200
                ? "#f87171"
                : t.rating < 1400
                  ? "#fbbf24"
                  : "#34d399",
            borderRadius: [0, 4, 4, 0],
          },
        })),
        barWidth: 12,
        label: {
          show: true,
          position: "right",
          color: "#8b9bb0",
          fontSize: 10,
          formatter: "{c}",
        },
      },
    ],
  };
}
