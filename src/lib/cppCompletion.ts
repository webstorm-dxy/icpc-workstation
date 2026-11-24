import type { Completion, CompletionContext, CompletionResult, CompletionSource } from "@codemirror/autocomplete";

const KEYWORDS = [
  "alignas", "alignof", "and", "asm", "auto", "bool", "break", "case", "catch",
  "char", "class", "const", "constexpr", "const_cast", "continue", "decltype",
  "default", "delete", "do", "double", "dynamic_cast", "else", "enum", "explicit",
  "export", "extern", "false", "float", "for", "friend", "goto", "if", "inline",
  "int", "long", "mutable", "namespace", "new", "noexcept", "not", "nullptr",
  "operator", "or", "private", "protected", "public", "register", "reinterpret_cast",
  "return", "short", "signed", "sizeof", "static", "static_assert", "static_cast",
  "struct", "switch", "template", "this", "throw", "true", "try", "typedef",
  "typeid", "typename", "union", "unsigned", "using", "virtual", "void",
  "volatile", "while", "xor",
];

const TYPES = [
  "int8_t", "int16_t", "int32_t", "int64_t", "uint8_t", "uint16_t", "uint32_t",
  "uint64_t", "size_t", "ptrdiff_t", "string", "vector", "array", "deque", "list",
  "forward_list", "set", "multiset", "map", "multimap", "unordered_set",
  "unordered_map", "unordered_multiset", "unordered_multimap", "stack", "queue",
  "priority_queue", "pair", "tuple", "bitset", "complex", "optional", "variant",
  "function", "shared_ptr", "unique_ptr", "weak_ptr", "string_view",
  "istream", "ostream", "stringstream", "ifstream", "ofstream",
];

const STL = [
  "std::", "cin", "cout", "cerr", "endl", "ios::sync_with_stdio", "cin.tie",
  "push_back", "pop_back", "emplace_back", "emplace", "insert", "erase", "clear",
  "size", "empty", "begin", "end", "rbegin", "rend", "front", "back", "at",
  "reserve", "resize", "assign", "find", "count", "lower_bound", "upper_bound",
  "equal_range", "sort", "stable_sort", "reverse", "unique", "binary_search",
  "max_element", "min_element", "accumulate", "iota", "fill", "copy", "swap",
  "min", "max", "abs", "gcd", "lcm", "pow", "sqrt", "log", "floor", "ceil",
  "to_string", "stoi", "stoll", "substr", "getline", "make_pair", "make_tuple",
  "tie", "get", "memset", "memcpy", "next_permutation", "prev_permutation",
  "priority_queue", "greater", "less", "hash", "INT_MAX", "INT_MIN", "LLONG_MAX",
  "LLONG_MIN", "M_PI", "1e9", "1LL",
];

const SNIPPETS: Completion[] = [
  {
    label: "fori",
    type: "snippet",
    detail: "for (int i = 0; i < n; i++)",
    apply: "for (int i = 0; i < n; i++) {\n    \n}",
  },
  {
    label: "forj",
    type: "snippet",
    detail: "for (int j = 0; j < m; j++)",
    apply: "for (int j = 0; j < m; j++) {\n    \n}",
  },
  {
    label: "fastio",
    type: "snippet",
    detail: "关闭同步的快速 IO",
    apply:
      "ios::sync_with_stdio(false);\ncin.tie(nullptr);\n",
  },
  {
    label: "main",
    type: "snippet",
    detail: "竞赛主函数模板",
    apply:
      "#include <bits/stdc++.h>\nusing namespace std;\n\nint main() {\n    ios::sync_with_stdio(false);\n    cin.tie(nullptr);\n\n    \n    return 0;\n}",
  },
  {
    label: "sortv",
    type: "snippet",
    detail: "对 vector 排序",
    apply: "sort(v.begin(), v.end());",
  },
  {
    label: "sortcmp",
    type: "snippet",
    detail: "带比较器的排序",
    apply: "sort(v.begin(), v.end(), [](const auto& a, const auto& b) {\n    return a < b;\n});",
  },
  {
    label: "dsu",
    type: "snippet",
    detail: "并查集",
    apply:
      "struct DSU {\n    vector<int> p, sz;\n    explicit DSU(int n) : p(n), sz(n, 1) { iota(p.begin(), p.end(), 0); }\n    int find(int x) { return p[x] == x ? x : p[x] = find(p[x]); }\n    bool unite(int a, int b) {\n        a = find(a); b = find(b);\n        if (a == b) return false;\n        if (sz[a] < sz[b]) swap(a, b);\n        p[b] = a; sz[a] += sz[b];\n        return true;\n    }\n};",
  },
  {
    label: "segtree",
    type: "snippet",
    detail: "区间和线段树（点修改/区间查询）",
    apply:
      "struct SegTree {\n    int n; vector<long long> t;\n    explicit SegTree(int n) : n(n), t(2 * n) {}\n    void update(int i, long long v) {\n        for (t[i += n] = v; i > 1; i >>= 1) t[i >> 1] = t[i] + t[i ^ 1];\n    }\n    long long query(int l, int r) {\n        long long res = 0;\n        for (l += n, r += n; l < r; l >>= 1, r >>= 1) {\n            if (l & 1) res += t[l++];\n            if (r & 1) res += t[--r];\n        }\n        return res;\n    }\n};",
  },
  {
    label: "dijkstra",
    type: "snippet",
    detail: "Dijkstra 最短路",
    apply:
      "auto dijkstra = [&](int s) {\n    vector<long long> dist(n + 1, LLONG_MAX);\n    priority_queue<pair<long long,int>, vector<pair<long long,int>>, greater<>> pq;\n    dist[s] = 0; pq.push({0, s});\n    while (!pq.empty()) {\n        auto [d, u] = pq.top(); pq.pop();\n        if (d != dist[u]) continue;\n        for (auto [v, w] : g[u]) if (dist[v] > d + w) {\n            dist[v] = d + w; pq.push({dist[v], v});\n        }\n    }\n    return dist;\n};",
  },
  {
    label: "kmp",
    type: "snippet",
    detail: "KMP 前缀函数",
    apply:
      "vector<int> pi(s.size());\nfor (int i = 1; i < (int)s.size(); i++) {\n    int j = pi[i - 1];\n    while (j > 0 && s[i] != s[j]) j = pi[j - 1];\n    if (s[i] == s[j]) j++;\n    pi[i] = j;\n}",
  },
  {
    label: "sieve",
    type: "snippet",
    detail: "线性筛/埃氏筛",
    apply:
      "vector<int> primes;\nvector<bool> is_prime(n + 1, true);\nis_prime[0] = is_prime[1] = false;\nfor (int i = 2; i <= n; i++) {\n    if (is_prime[i]) primes.push_back(i);\n    for (int p : primes) {\n        if (1LL * i * p > n) break;\n        is_prime[i * p] = false;\n        if (i % p == 0) break;\n    }\n}",
  },
  {
    label: "powmod",
    type: "snippet",
    detail: "快速幂",
    apply:
      "long long powmod(long long a, long long b) {\n    long long res = 1; a %= MOD;\n    while (b) { if (b & 1) res = res * a % MOD; a = a * a % MOD; b >>= 1; }\n    return res;\n}",
  },
  {
    label: "grid",
    type: "snippet",
    detail: "网格方向数组",
    apply: "const int dx[] = {0, 0, 1, -1};\nconst int dy[] = {1, -1, 0, 0};",
  },
  {
    label: "read",
    type: "snippet",
    detail: "读入 n 与数组",
    apply: "int n; cin >> n;\nvector<int> a(n);\nfor (auto& x : a) cin >> x;",
  },
  {
    label: "print",
    type: "snippet",
    detail: "输出 vector",
    apply: "for (int i = 0; i < (int)ans.size(); i++) cout << ans[i] << \" \\n\"[i + 1 == (int)ans.size()];",
  },
];

function documentWords(context: CompletionContext): Completion[] {
  const text = context.state.doc.toString();
  const words = new Map<string, number>();
  for (const match of text.matchAll(/[A-Za-z_][A-Za-z0-9_]{2,}/g)) {
    const word = match[0];
    words.set(word, (words.get(word) ?? 0) + 1);
  }
  return Array.from(words.entries())
    .filter(([word]) => word.length >= 3 && !TYPES.includes(word) && !KEYWORDS.includes(word))
    .slice(0, 180)
    .map(([word, count]) => ({
      label: word,
      type: "variable",
      detail: `文档中出现 ${count} 次`,
      boost: Math.min(count, 5),
    }));
}

export function buildCppOptions(): Completion[] {
  const keywords: Completion[] = KEYWORDS.map((label) => ({
    label,
    type: "keyword",
    boost: 2,
  }));
  const types: Completion[] = TYPES.map((label) => ({
    label,
    type: "type",
    detail: "标准库类型",
    boost: 6,
  }));
  const stl: Completion[] = STL.map((label) => ({
    label,
    type: "function",
    detail: "C++ 标准库",
    boost: 4,
  }));
  return [...SNIPPETS, ...types, ...stl, ...keywords];
}

const BASE_OPTIONS = buildCppOptions();

export const cppCompletion: CompletionSource = (
  context: CompletionContext,
): CompletionResult | null => {
  const word = context.matchBefore(/[A-Za-z_][\w:]*$/);
  if (!word && !context.explicit) return null;
  const prefix = word?.text ?? "";
  const options = [...BASE_OPTIONS, ...documentWords(context)];
  const filtered = prefix
    ? options.filter((option) =>
        String(option.label).toLowerCase().startsWith(prefix.toLowerCase()),
      )
    : options;
  return {
    from: word ? word.from : context.pos,
    options: filtered.slice(0, 80),
    validFor: /^[\w:]*$/,
  };
};
