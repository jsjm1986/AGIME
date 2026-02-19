// Parse and analyze task execution results

export interface ToolCall {
  id: string;
  toolName: string;
  mcpServer: string;
  callId: string;
  timestamp: Date;
  success: boolean;
}

export interface ExecutionStats {
  totalToolCalls: number;
  toolUsage: Record<string, number>;
  mcpServers: string[];
  startTime: Date | null;
  endTime: Date | null;
  duration: number | null;
}

export interface ParsedResult {
  type: 'message' | 'tool_call' | 'error' | 'final_result';
  content: string;
  toolCalls: ToolCall[];
  timestamp: Date;
}

// Parse tool calls from message text
// Format: [Tool: mcp_server__tool_name][Result: call_xxx]
export function parseToolCalls(text: string): ToolCall[] {
  const toolCalls: ToolCall[] = [];
  const regex = /\[Tool:\s*([^\]]+)\]\[Result:\s*([^\]]+)\]/g;
  let match;

  while ((match = regex.exec(text)) !== null) {
    const fullToolName = match[1].trim();
    const callId = match[2].trim();

    // Split tool name into mcp_server and tool_name
    const parts = fullToolName.split('__');
    const mcpServer = parts.length > 1 ? parts[0] : 'default';
    const toolName = parts.length > 1 ? parts.slice(1).join('__') : fullToolName;

    toolCalls.push({
      id: `${callId}-${toolCalls.length}`,
      toolName,
      mcpServer,
      callId,
      timestamp: new Date(),
      success: true,
    });
  }

  return toolCalls;
}

// Remove tool call markers from text for clean display
export function cleanMessageText(text: string): string {
  return text
    .replace(/\[Tool:\s*[^\]]+\]\[Result:\s*[^\]]+\]/g, '')
    .replace(/\s+/g, ' ')
    .trim();
}

// Calculate execution statistics
export function calculateStats(results: ParsedResult[]): ExecutionStats {
  const allToolCalls: ToolCall[] = [];
  const toolUsage: Record<string, number> = {};
  const mcpServersSet = new Set<string>();

  let startTime: Date | null = null;
  let endTime: Date | null = null;

  for (const result of results) {
    // Track time range
    if (!startTime || result.timestamp < startTime) {
      startTime = result.timestamp;
    }
    if (!endTime || result.timestamp > endTime) {
      endTime = result.timestamp;
    }

    // Collect tool calls
    for (const call of result.toolCalls) {
      allToolCalls.push(call);
      mcpServersSet.add(call.mcpServer);

      const key = `${call.mcpServer}__${call.toolName}`;
      toolUsage[key] = (toolUsage[key] || 0) + 1;
    }
  }

  const duration = startTime && endTime
    ? endTime.getTime() - startTime.getTime()
    : null;

  return {
    totalToolCalls: allToolCalls.length,
    toolUsage,
    mcpServers: Array.from(mcpServersSet),
    startTime,
    endTime,
    duration,
  };
}

// Format duration for display
export function formatDuration(ms: number | null): string {
  if (ms === null) return '-';
  if (ms < 1000) return `${ms}ms`;
  if (ms < 60000) return `${(ms / 1000).toFixed(1)}s`;
  return `${Math.floor(ms / 60000)}m ${Math.floor((ms % 60000) / 1000)}s`;
}
