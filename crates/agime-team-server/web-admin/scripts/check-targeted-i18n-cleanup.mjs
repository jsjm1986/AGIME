import fs from "node:fs";
import path from "node:path";

const projectRoot = process.cwd();

const targets = [
  "src/components/chat/RelationshipMemoryControl.tsx",
  "src/components/chat/ChatConversation.tsx",
  "src/components/agent/AgentDocumentPanel.tsx",
  "src/components/agent/ExecutionPolicyPanel.tsx",
  "src/components/team/DocumentsTab.tsx",
  "src/components/team/DigitalAvatarSection.tsx",
  "src/components/team/ExperimentSection.tsx",
  "src/components/team/ExternalUsersTab.tsx",
  "src/components/team/ScheduledTasksSection.tsx",
  "src/components/team/SettingsTab.tsx",
  "src/components/team/TeamChannelsPanel.tsx",
  "src/components/team/experiment/AutomationLabWorkspace.tsx",
  "src/components/team/experiment/ExperimentHome.tsx",
  "src/components/team/experiment/labRegistry.ts",
  "src/pages/AvatarAgentManagerPage.tsx",
];

const allowlist = [
  "/用户上传文档",
  'normalized.includes("api 文档")',
  'normalized.includes("接口文档")',
  'name: "Agentify｜万物智能"',
  'tagline: "把多个软件系统接成一个可持续对话的 Agent 应用"',
  "导入 API 资料，用对话生成可持续对话、可执行、可长期运行的 Agent App，让多个软件系统像一个智能体一样协同工作。",
  'featuredMetric: "首个可用智能应用"',
  `{ value: "1", zh: "周一", en: "Mon" }`,
  `{ value: "2", zh: "周二", en: "Tue" }`,
  `{ value: "3", zh: "周三", en: "Wed" }`,
  `{ value: "4", zh: "周四", en: "Thu" }`,
  `{ value: "5", zh: "周五", en: "Fri" }`,
  `{ value: "6", zh: "周六", en: "Sat" }`,
  `{ value: "0", zh: "周日", en: "Sun" }`,
  `'管理Agent'`,
  `'管理 Agent'`,
  `'分身管理Agent'`,
  `'分身管理 Agent'`,
  `'服务Agent'`,
  `'服务 Agent'`,
  `'分身服务Agent'`,
  `'分身服务 Agent'`,
  `replace(/(?:管理\\s*Agent)(?:\\s*[-－—]?\\s*管理\\s*Agent)+$/giu, '管理Agent')`,
  `replace(/(?:分身管理\\s*Agent)(?:\\s*[-－—]?\\s*分身管理\\s*Agent)+$/giu, '分身管理Agent')`,
  `replace(/(?:服务\\s*Agent)(?:\\s*[-－—]?\\s*服务\\s*Agent)+$/giu, '服务Agent')`,
  `replace(/(?:分身服务\\s*Agent)(?:\\s*[-－—]?\\s*分身服务\\s*Agent)+$/giu, '分身服务Agent')`,
  `joined.includes('高风险')`,
  `joined.includes('严重')`,
  `joined.includes('中风险')`,
];

const diagnostics = [];

for (const relativePath of targets) {
  const absolutePath = path.join(projectRoot, relativePath);
  const source = fs.readFileSync(absolutePath, "utf8");
  const lines = source.split(/\r?\n/);
  lines.forEach((line, index) => {
    if (!/[\p{Script=Han}]/u.test(line)) {
      return;
    }
    const contextWindow = [lines[index - 1] || "", line, lines[index + 1] || ""].join("\n");
    if (
      /^\s*(\/\/|\*|\/\*|import\s)/.test(line) ||
      /(?:t|translate|bilingual)\(/.test(contextWindow) ||
      allowlist.some((snippet) => line.includes(snippet))
    ) {
      return;
    }
    diagnostics.push(`${relativePath}:${index + 1}: ${line.trim()}`);
  });
}

if (diagnostics.length > 0) {
  console.error("Found raw Chinese strings in targeted bilingual surfaces:");
  for (const item of diagnostics) {
    console.error(`- ${item}`);
  }
  process.exit(1);
}

console.log("No raw Chinese UI strings found in targeted bilingual surfaces.");
