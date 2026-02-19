import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { agentApi, TeamAgent } from '../../api/agent';
import type { ApprovalPolicy, ExecutionMode } from '../../api/mission';
import { DocumentPicker } from '../documents/DocumentPicker';
import type { DocumentSummary } from '../../api/documents';

interface CreateMissionDialogProps {
  teamId: string;
  open: boolean;
  onClose: () => void;
  onCreate: (data: {
    agent_id: string;
    goal: string;
    context?: string;
    approval_policy: ApprovalPolicy;
    token_budget?: number;
    execution_mode?: ExecutionMode;
    attached_document_ids?: string[];
  }) => void;
}

export function CreateMissionDialog({
  teamId,
  open,
  onClose,
  onCreate,
}: CreateMissionDialogProps) {
  const { t } = useTranslation();
  const [agents, setAgents] = useState<TeamAgent[]>([]);
  const [agentId, setAgentId] = useState('');
  const [goal, setGoal] = useState('');
  const [context, setContext] = useState('');
  const [policy, setPolicy] = useState<ApprovalPolicy>('auto');
  const [executionMode, setExecutionMode] = useState<ExecutionMode>('sequential');
  const [budget, setBudget] = useState('');
  const [loading, setLoading] = useState(false);
  const [attachedDocs, setAttachedDocs] = useState<DocumentSummary[]>([]);
  const [showDocPicker, setShowDocPicker] = useState(false);

  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    agentApi.listAgents(teamId).then(res => {
      if (!cancelled) {
        setAgents(res.items || []);
        if (res.items?.length && !agentId) setAgentId(res.items[0].id);
      }
    });
    return () => { cancelled = true; };
  }, [teamId, open]);

  if (!open) return null;

  const handleSubmit = () => {
    if (!agentId || !goal.trim()) return;
    setLoading(true);
    onCreate({
      agent_id: agentId,
      goal: goal.trim(),
      context: context.trim() || undefined,
      approval_policy: policy,
      token_budget: budget ? parseInt(budget, 10) : undefined,
      execution_mode: executionMode,
      attached_document_ids: attachedDocs.length > 0 ? attachedDocs.map(d => d.id) : undefined,
    });
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
      <div className="bg-background rounded-lg shadow-xl w-full max-w-lg mx-4 p-6">
        <h2 className="text-lg font-semibold mb-4">{t('mission.create')}</h2>

        {/* Agent select */}
        <div className="mb-3">
          <label className="block text-sm font-medium mb-1">Agent</label>
          <select
            value={agentId}
            onChange={e => setAgentId(e.target.value)}
            className="w-full rounded-md border px-3 py-2 text-sm bg-background"
          >
            {agents.map(a => (
              <option key={a.id} value={a.id}>{a.name}</option>
            ))}
          </select>
        </div>

        {/* Goal */}
        <div className="mb-3">
          <label className="block text-sm font-medium mb-1">{t('mission.goal')}</label>
          <textarea
            value={goal}
            onChange={e => setGoal(e.target.value)}
            rows={3}
            className="w-full rounded-md border px-3 py-2 text-sm bg-background resize-none"
            placeholder={t('mission.goal')}
          />
        </div>

        {/* Context */}
        <div className="mb-3">
          <label className="block text-sm font-medium mb-1">{t('mission.context')}</label>
          <textarea
            value={context}
            onChange={e => setContext(e.target.value)}
            rows={2}
            className="w-full rounded-md border px-3 py-2 text-sm bg-background resize-none"
          />
        </div>

        {/* Approval policy */}
        <div className="mb-3">
          <label className="block text-sm font-medium mb-1">{t('mission.approvalPolicy')}</label>
          <select
            value={policy}
            onChange={e => setPolicy(e.target.value as ApprovalPolicy)}
            className="w-full rounded-md border px-3 py-2 text-sm bg-background"
          >
            <option value="auto">{t('mission.auto')}</option>
            <option value="checkpoint">{t('mission.checkpoint')}</option>
            <option value="manual">{t('mission.manual')}</option>
          </select>
        </div>

        {/* Execution mode */}
        <div className="mb-3">
          <label className="block text-sm font-medium mb-1">{t('mission.executionMode', 'Execution Mode')}</label>
          <select
            value={executionMode}
            onChange={e => setExecutionMode(e.target.value as ExecutionMode)}
            className="w-full rounded-md border px-3 py-2 text-sm bg-background"
          >
            <option value="sequential">{t('mission.sequential', 'Sequential')}</option>
            <option value="adaptive">{t('mission.adaptive', 'Adaptive (Goal Tree)')}</option>
          </select>
        </div>

        {/* Token budget */}
        <div className="mb-3">
          <label className="block text-sm font-medium mb-1">{t('mission.tokenBudget')}</label>
          <input
            type="number"
            value={budget}
            onChange={e => setBudget(e.target.value)}
            className="w-full rounded-md border px-3 py-2 text-sm bg-background"
            placeholder={t('mission.unlimited')}
            min={0}
          />
        </div>

        {/* Attached documents */}
        <div className="mb-4">
          <label className="block text-sm font-medium mb-1">{t('documents.attachDocuments')}</label>
          <div className="flex flex-wrap gap-1 mb-2">
            {attachedDocs.map(doc => (
              <span key={doc.id} className="inline-flex items-center gap-1 text-xs bg-muted px-2 py-1 rounded-full">
                {doc.display_name || doc.name}
                <button onClick={() => setAttachedDocs(prev => prev.filter(d => d.id !== doc.id))}>
                  &times;
                </button>
              </span>
            ))}
          </div>
          <button
            type="button"
            onClick={() => setShowDocPicker(true)}
            className="text-xs text-primary hover:underline"
          >
            + {t('documents.selectDocuments')}
          </button>
        </div>

        {/* Actions */}
        <div className="flex justify-end gap-2">
          <button
            onClick={onClose}
            className="px-4 py-2 text-sm rounded-md border hover:bg-accent"
          >
            {t('common.cancel', 'Cancel')}
          </button>
          <button
            onClick={handleSubmit}
            disabled={!agentId || !goal.trim() || loading}
            className="px-4 py-2 text-sm rounded-md bg-primary text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
          >
            {t('mission.create')}
          </button>
        </div>
      </div>

      {/* Document Picker */}
      <DocumentPicker
        teamId={teamId}
        open={showDocPicker}
        onClose={() => setShowDocPicker(false)}
        onSelect={(docs) => setAttachedDocs(docs)}
        selectedIds={attachedDocs.map(d => d.id)}
      />
    </div>
  );
}
