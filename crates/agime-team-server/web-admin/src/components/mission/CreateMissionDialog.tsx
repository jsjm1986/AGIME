import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { agentApi, TeamAgent } from '../../api/agent';
import type { ApprovalPolicy } from '../../api/mission';
import { DocumentPicker } from '../documents/DocumentPicker';
import type { DocumentSummary } from '../../api/documents';

function isMissionSelectableAgent(agent: TeamAgent): boolean {
  const domain = agent.agent_domain ?? 'general';
  const role = agent.agent_role ?? 'default';
  return domain === 'general' && role === 'default';
}

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
  const [budget, setBudget] = useState('');
  const [loading, setLoading] = useState(false);
  const [attachedDocs, setAttachedDocs] = useState<DocumentSummary[]>([]);
  const [showDocPicker, setShowDocPicker] = useState(false);

  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    agentApi.listAgents(teamId).then(res => {
      if (!cancelled) {
        const missionAgents = (res.items || []).filter(isMissionSelectableAgent);
        setAgents(missionAgents);
        if (!missionAgents.some(agent => agent.id === agentId)) {
          setAgentId(missionAgents[0]?.id || '');
        }
      }
    });
    return () => { cancelled = true; };
  }, [teamId, open, agentId]);

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
      attached_document_ids: attachedDocs.length > 0 ? attachedDocs.map(d => d.id) : undefined,
    });
  };

  return (
    <div className="fixed inset-0 z-50 overflow-y-auto bg-black/50 p-3 sm:flex sm:items-center sm:justify-center">
      <div className="mx-auto my-3 w-full max-w-lg rounded-2xl bg-background p-4 shadow-xl sm:my-6 sm:p-6">
        <h2 className="text-lg font-semibold mb-4">{t('mission.create')}</h2>

        {/* Agent select */}
        <div className="mb-3">
          <label className="block text-sm font-medium mb-1">
            {t('mission.agent', 'Agent')}
          </label>
          <select
            value={agentId}
            onChange={e => setAgentId(e.target.value)}
            className="w-full rounded-md border px-3 py-2 text-sm bg-background"
          >
            {agents.length === 0 && (
              <option value="">{t('mission.noGeneralAgents', 'No general-purpose agents available')}</option>
            )}
            {agents.map(a => (
              <option key={a.id} value={a.id}>{a.name}</option>
            ))}
          </select>
          <p className="mt-1 text-xs text-muted-foreground">
            {t(
              'mission.generalAgentOnlyHint',
              'Only standard general-purpose agents can run V4 mission tasks. Derived avatar, manager, service, and ecosystem agents are excluded.',
            )}
          </p>
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

        {/* Additional constraints */}
        <div className="mb-3">
          <label className="block text-sm font-medium mb-1">
            {t('mission.constraints', 'Additional Constraints (Optional)')}
          </label>
          <textarea
            value={context}
            onChange={e => setContext(e.target.value)}
            rows={2}
            className="w-full rounded-md border px-3 py-2 text-sm bg-background resize-none"
            placeholder={t(
              'mission.constraintsPlaceholder',
              'Example: Keep output within 10 slides, cite public sources, and avoid fabricated data.',
            )}
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
        <div className="flex flex-col-reverse gap-2 pt-1 sm:flex-row sm:justify-end">
          <button
            onClick={onClose}
            className="w-full rounded-md border px-4 py-2 text-sm hover:bg-accent sm:w-auto"
          >
            {t('common.cancel', 'Cancel')}
          </button>
          <button
            onClick={handleSubmit}
            disabled={!agentId || !goal.trim() || loading}
            className="w-full rounded-md bg-primary px-4 py-2 text-sm text-primary-foreground hover:bg-primary/90 disabled:opacity-50 sm:w-auto"
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
