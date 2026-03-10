export type DecisionMode =
  | 'approve_direct'
  | 'approve_sandbox'
  | 'require_human_confirm'
  | 'deny';

export type GapRequestStatus = 'pending' | 'approved' | 'needs_human' | 'rejected';

export interface CapabilityGapRequest {
  id: string;
  title: string;
  detail: string;
  requestedScope: string[];
  risk: 'low' | 'medium' | 'high';
  status: GapRequestStatus;
  source: 'avatar' | 'manager' | 'user';
  createdAt: string;
  updatedAt: string;
  decision?: DecisionMode;
  decisionReason?: string;
}

export type ProposalStatus = 'draft' | 'pending_approval' | 'approved' | 'rejected' | 'pilot' | 'active';

export interface AgentGapProposal {
  id: string;
  title: string;
  description: string;
  expectedGain: string;
  status: ProposalStatus;
  proposedBy: 'avatar' | 'manager';
  createdAt: string;
  updatedAt: string;
}

export type OptimizationProblemType = 'prompt' | 'tool' | 'skill' | 'policy' | 'team_prompt';
export type OptimizationStatus = 'pending' | 'approved' | 'rejected' | 'experimenting' | 'deployed' | 'rolled_back';
export type RuntimeLogStatus = 'pending' | 'ticketed' | 'requested' | 'dismissed';

export interface OptimizationTicket {
  id: string;
  title: string;
  problemType: OptimizationProblemType;
  evidence: string;
  proposal: string;
  expectedGain: string;
  risk: 'low' | 'medium' | 'high';
  status: OptimizationStatus;
  createdAt: string;
  updatedAt: string;
}

export interface RuntimeLogEntry {
  id: string;
  title: string;
  evidence: string;
  proposal: string;
  expectedGain: string;
  risk: 'low' | 'medium' | 'high';
  problemType: OptimizationProblemType;
  status: RuntimeLogStatus;
  createdAt: string;
}

export interface AvatarGovernanceState {
  capabilityRequests: CapabilityGapRequest[];
  gapProposals: AgentGapProposal[];
  optimizationTickets: OptimizationTicket[];
  runtimeLogs: RuntimeLogEntry[];
}

export interface AvatarGovernanceAutomationConfig {
  autoProposalTriggerCount: number;
  managerApprovalMode: 'manager_decides' | 'human_gate';
  optimizationMode: 'dual_loop' | 'manager_only';
  lowRiskAction: 'auto_execute' | 'manager_review' | 'human_review';
  mediumRiskAction: 'auto_execute' | 'manager_review' | 'human_review';
  highRiskAction: 'auto_execute' | 'manager_review' | 'human_review';
  autoCreateCapabilityRequests: boolean;
  autoCreateOptimizationTickets: boolean;
  requireHumanForPublish: boolean;
}

export const DEFAULT_AUTOMATION_CONFIG: AvatarGovernanceAutomationConfig = {
  autoProposalTriggerCount: 3,
  managerApprovalMode: 'manager_decides',
  optimizationMode: 'dual_loop',
  lowRiskAction: 'auto_execute',
  mediumRiskAction: 'manager_review',
  highRiskAction: 'human_review',
  autoCreateCapabilityRequests: true,
  autoCreateOptimizationTickets: true,
  requireHumanForPublish: true,
};

export function createEmptyGovernanceState(): AvatarGovernanceState {
  return {
    capabilityRequests: [],
    gapProposals: [],
    optimizationTickets: [],
    runtimeLogs: [],
  };
}

function isRecord(input: unknown): input is Record<string, unknown> {
  return !!input && typeof input === 'object' && !Array.isArray(input);
}

function toStringArray(input: unknown): string[] {
  if (!Array.isArray(input)) return [];
  return input.filter((x): x is string => typeof x === 'string');
}

function toIntInRange(input: unknown, fallback: number, min: number, max: number): number {
  const n = typeof input === 'number' ? input : Number.parseInt(String(input ?? ''), 10);
  if (!Number.isFinite(n)) return fallback;
  const v = Math.round(n);
  if (v < min) return min;
  if (v > max) return max;
  return v;
}

function toRisk(input: unknown): 'low' | 'medium' | 'high' {
  return input === 'low' || input === 'high' ? input : 'medium';
}

function toBoolean(input: unknown, fallback: boolean): boolean {
  if (typeof input === 'boolean') return input;
  if (typeof input === 'string') {
    const normalized = input.trim().toLowerCase();
    if (normalized === 'true') return true;
    if (normalized === 'false') return false;
  }
  return fallback;
}

function toChoice<T extends string>(input: unknown, fallback: T, allowed: readonly T[]): T {
  if (typeof input !== 'string') return fallback;
  const normalized = input.trim() as T;
  return allowed.includes(normalized) ? normalized : fallback;
}

function toNowIso(): string {
  return new Date().toISOString();
}

export function readGovernanceState(settings: Record<string, unknown> | null | undefined): AvatarGovernanceState {
  if (!settings) return createEmptyGovernanceState();
  const raw = settings.digitalAvatarGovernance;
  if (!isRecord(raw)) return createEmptyGovernanceState();

  const capabilityRequests = Array.isArray(raw.capabilityRequests)
    ? raw.capabilityRequests
        .filter(isRecord)
        .map((item): CapabilityGapRequest | null => {
          const id = typeof item.id === 'string' ? item.id : '';
          const title = typeof item.title === 'string' ? item.title : '';
          if (!id || !title) return null;
          const status: GapRequestStatus =
            item.status === 'approved'
              ? 'approved'
              : item.status === 'needs_human'
              ? 'needs_human'
              : item.status === 'rejected'
              ? 'rejected'
              : 'pending';
          return {
            id,
            title,
            detail: typeof item.detail === 'string' ? item.detail : '',
            requestedScope: toStringArray(item.requestedScope),
            risk: toRisk(item.risk),
            status,
            source: item.source === 'avatar' || item.source === 'user' ? item.source : 'manager',
            createdAt: typeof item.createdAt === 'string' ? item.createdAt : toNowIso(),
            updatedAt: typeof item.updatedAt === 'string' ? item.updatedAt : toNowIso(),
            decision:
              item.decision === 'approve_direct' ||
              item.decision === 'approve_sandbox' ||
              item.decision === 'require_human_confirm' ||
              item.decision === 'deny'
                ? item.decision
                : undefined,
            decisionReason: typeof item.decisionReason === 'string' ? item.decisionReason : undefined,
          };
        })
        .filter((x): x is CapabilityGapRequest => x !== null)
    : [];

  const gapProposals = Array.isArray(raw.gapProposals)
    ? raw.gapProposals
        .filter(isRecord)
        .map((item): AgentGapProposal | null => {
          const id = typeof item.id === 'string' ? item.id : '';
          const title = typeof item.title === 'string' ? item.title : '';
          if (!id || !title) return null;
          const status: ProposalStatus =
            item.status === 'pending_approval' ||
            item.status === 'approved' ||
            item.status === 'rejected' ||
            item.status === 'pilot' ||
            item.status === 'active'
              ? item.status
              : 'draft';
          return {
            id,
            title,
            description: typeof item.description === 'string' ? item.description : '',
            expectedGain: typeof item.expectedGain === 'string' ? item.expectedGain : '',
            status,
            proposedBy: item.proposedBy === 'avatar' ? 'avatar' : 'manager',
            createdAt: typeof item.createdAt === 'string' ? item.createdAt : toNowIso(),
            updatedAt: typeof item.updatedAt === 'string' ? item.updatedAt : toNowIso(),
          };
        })
        .filter((x): x is AgentGapProposal => x !== null)
    : [];

  const optimizationTickets = Array.isArray(raw.optimizationTickets)
    ? raw.optimizationTickets
        .filter(isRecord)
        .map((item): OptimizationTicket | null => {
          const id = typeof item.id === 'string' ? item.id : '';
          const title = typeof item.title === 'string' ? item.title : '';
          if (!id || !title) return null;
          const status: OptimizationStatus =
            item.status === 'approved' ||
            item.status === 'rejected' ||
            item.status === 'experimenting' ||
            item.status === 'deployed' ||
            item.status === 'rolled_back'
              ? item.status
              : 'pending';
          const problemType: OptimizationProblemType =
            item.problemType === 'tool' ||
            item.problemType === 'skill' ||
            item.problemType === 'policy' ||
            item.problemType === 'team_prompt'
              ? item.problemType
              : 'prompt';
          return {
            id,
            title,
            problemType,
            evidence: typeof item.evidence === 'string' ? item.evidence : '',
            proposal: typeof item.proposal === 'string' ? item.proposal : '',
            expectedGain: typeof item.expectedGain === 'string' ? item.expectedGain : '',
            risk: toRisk(item.risk),
            status,
            createdAt: typeof item.createdAt === 'string' ? item.createdAt : toNowIso(),
            updatedAt: typeof item.updatedAt === 'string' ? item.updatedAt : toNowIso(),
          };
        })
        .filter((x): x is OptimizationTicket => x !== null)
    : [];

  const runtimeLogs = Array.isArray(raw.runtimeLogs)
    ? raw.runtimeLogs
        .filter(isRecord)
        .map((item): RuntimeLogEntry | null => {
          const id = typeof item.id === 'string' ? item.id : '';
          const title = typeof item.title === 'string' ? item.title : '';
          if (!id || !title) return null;
          const status: RuntimeLogStatus =
            item.status === 'ticketed' ||
            item.status === 'requested' ||
            item.status === 'dismissed'
              ? item.status
              : 'pending';
          const problemType: OptimizationProblemType =
            item.problemType === 'tool' ||
            item.problemType === 'skill' ||
            item.problemType === 'policy' ||
            item.problemType === 'team_prompt'
              ? item.problemType
              : 'prompt';
          return {
            id,
            title,
            evidence: typeof item.evidence === 'string' ? item.evidence : '',
            proposal: typeof item.proposal === 'string' ? item.proposal : '',
            expectedGain: typeof item.expectedGain === 'string' ? item.expectedGain : '',
            risk: toRisk(item.risk),
            problemType,
            status,
            createdAt: typeof item.createdAt === 'string' ? item.createdAt : toNowIso(),
          };
        })
        .filter((x): x is RuntimeLogEntry => x !== null)
    : [];

  return {
    capabilityRequests,
    gapProposals,
    optimizationTickets,
    runtimeLogs,
  };
}

export function readGovernanceAutomationConfig(
  settings: Record<string, unknown> | null | undefined,
): AvatarGovernanceAutomationConfig {
  if (!settings) return DEFAULT_AUTOMATION_CONFIG;

  const fromTop = settings.digitalAvatarGovernanceConfig;
  const fromGovernance = isRecord(settings.digitalAvatarGovernance)
    ? settings.digitalAvatarGovernance.config
    : undefined;
  const raw = isRecord(fromTop) ? fromTop : isRecord(fromGovernance) ? fromGovernance : null;
  if (!raw) return DEFAULT_AUTOMATION_CONFIG;

  return {
    autoProposalTriggerCount: toIntInRange(
      raw.autoProposalTriggerCount,
      DEFAULT_AUTOMATION_CONFIG.autoProposalTriggerCount,
      1,
      10,
    ),
    managerApprovalMode: toChoice(
      raw.managerApprovalMode ?? (isRecord(settings) ? settings.managerApprovalMode : undefined),
      DEFAULT_AUTOMATION_CONFIG.managerApprovalMode,
      ['manager_decides', 'human_gate'] as const,
    ),
    optimizationMode: toChoice(
      raw.optimizationMode ?? (isRecord(settings) ? settings.optimizationMode : undefined),
      DEFAULT_AUTOMATION_CONFIG.optimizationMode,
      ['dual_loop', 'manager_only'] as const,
    ),
    lowRiskAction: toChoice(
      raw.lowRiskAction,
      DEFAULT_AUTOMATION_CONFIG.lowRiskAction,
      ['auto_execute', 'manager_review', 'human_review'] as const,
    ),
    mediumRiskAction: toChoice(
      raw.mediumRiskAction,
      DEFAULT_AUTOMATION_CONFIG.mediumRiskAction,
      ['auto_execute', 'manager_review', 'human_review'] as const,
    ),
    highRiskAction: toChoice(
      raw.highRiskAction,
      DEFAULT_AUTOMATION_CONFIG.highRiskAction,
      ['auto_execute', 'manager_review', 'human_review'] as const,
    ),
    autoCreateCapabilityRequests: toBoolean(
      raw.autoCreateCapabilityRequests,
      DEFAULT_AUTOMATION_CONFIG.autoCreateCapabilityRequests,
    ),
    autoCreateOptimizationTickets: toBoolean(
      raw.autoCreateOptimizationTickets,
      DEFAULT_AUTOMATION_CONFIG.autoCreateOptimizationTickets,
    ),
    requireHumanForPublish: toBoolean(
      raw.requireHumanForPublish,
      DEFAULT_AUTOMATION_CONFIG.requireHumanForPublish,
    ),
  };
}

export function mergeGovernanceSettings(
  base: Record<string, unknown> | null | undefined,
  governance: AvatarGovernanceState,
): Record<string, unknown> {
  const merged: Record<string, unknown> = base && isRecord(base) ? { ...base } : {};
  merged.digitalAvatarGovernance = {
    capabilityRequests: governance.capabilityRequests,
    gapProposals: governance.gapProposals,
    optimizationTickets: governance.optimizationTickets,
    runtimeLogs: governance.runtimeLogs,
  };
  return merged;
}

export function mergeGovernanceAutomationConfig(
  base: Record<string, unknown> | null | undefined,
  config: AvatarGovernanceAutomationConfig,
): Record<string, unknown> {
  const merged: Record<string, unknown> = base && isRecord(base) ? { ...base } : {};
  const automationConfig = {
    autoProposalTriggerCount: toIntInRange(config.autoProposalTriggerCount, 3, 1, 10),
    managerApprovalMode: toChoice(
      config.managerApprovalMode,
      DEFAULT_AUTOMATION_CONFIG.managerApprovalMode,
      ['manager_decides', 'human_gate'] as const,
    ),
    optimizationMode: toChoice(
      config.optimizationMode,
      DEFAULT_AUTOMATION_CONFIG.optimizationMode,
      ['dual_loop', 'manager_only'] as const,
    ),
    lowRiskAction: toChoice(
      config.lowRiskAction,
      DEFAULT_AUTOMATION_CONFIG.lowRiskAction,
      ['auto_execute', 'manager_review', 'human_review'] as const,
    ),
    mediumRiskAction: toChoice(
      config.mediumRiskAction,
      DEFAULT_AUTOMATION_CONFIG.mediumRiskAction,
      ['auto_execute', 'manager_review', 'human_review'] as const,
    ),
    highRiskAction: toChoice(
      config.highRiskAction,
      DEFAULT_AUTOMATION_CONFIG.highRiskAction,
      ['auto_execute', 'manager_review', 'human_review'] as const,
    ),
    autoCreateCapabilityRequests: toBoolean(
      config.autoCreateCapabilityRequests,
      DEFAULT_AUTOMATION_CONFIG.autoCreateCapabilityRequests,
    ),
    autoCreateOptimizationTickets: toBoolean(
      config.autoCreateOptimizationTickets,
      DEFAULT_AUTOMATION_CONFIG.autoCreateOptimizationTickets,
    ),
    requireHumanForPublish: toBoolean(
      config.requireHumanForPublish,
      DEFAULT_AUTOMATION_CONFIG.requireHumanForPublish,
    ),
  };
  merged.digitalAvatarGovernanceConfig = automationConfig;
  merged.managerApprovalMode = automationConfig.managerApprovalMode;
  merged.optimizationMode = automationConfig.optimizationMode;
  return merged;
}

export function makeId(prefix: string): string {
  const rand = Math.random().toString(36).slice(2, 8);
  return `${prefix}_${Date.now()}_${rand}`;
}
