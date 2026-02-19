import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import { Button } from '../ui/button';
import { missionApi } from '../../api/mission';
import type { DocumentSummary } from '../../api/documents';

interface QuickTaskMenuProps {
  teamId: string;
  agentId: string;
  document: DocumentSummary;
  onClose?: () => void;
}

interface QuickTemplate {
  key: string;
  labelKey: string;
  goal: string;
  category: string;
}

const templates: QuickTemplate[] = [
  { key: 'translateEn', labelKey: 'documents.quickTask.translateEn', goal: 'Translate this document to English', category: 'translation' },
  { key: 'translateZh', labelKey: 'documents.quickTask.translateZh', goal: '将此文档翻译为中文', category: 'translation' },
  { key: 'summarize', labelKey: 'documents.quickTask.summarize', goal: 'Generate a concise summary of this document', category: 'summary' },
  { key: 'review', labelKey: 'documents.quickTask.review', goal: 'Review this code and provide feedback', category: 'review' },
];

export function QuickTaskMenu({ teamId, agentId, document: doc, onClose }: QuickTaskMenuProps) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [submitting, setSubmitting] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const handleQuickTask = async (template: QuickTemplate) => {
    setSubmitting(template.key);
    setError(null);
    try {
      const res = await missionApi.createMission({
        agent_id: agentId,
        goal: `${template.goal}\n\nDocument: ${doc.display_name || doc.name}`,
        approval_policy: 'auto',
        attached_document_ids: [doc.id],
      });
      navigate(`/teams/${teamId}/missions/${res.mission_id}`);
      onClose?.();
    } catch (e) {
      console.error('Failed to create quick task:', e);
      setError(t('documents.quickTaskFailed'));
    } finally {
      setSubmitting(null);
    }
  };

  return (
    <div className="space-y-1 p-1">
      {error && (
        <p className="text-xs text-destructive px-2 py-1">{error}</p>
      )}
      {templates.map(tmpl => (
        <Button
          key={tmpl.key}
          variant="ghost"
          size="sm"
          className="w-full justify-start text-sm"
          disabled={submitting !== null}
          onClick={() => handleQuickTask(tmpl)}
        >
          {submitting === tmpl.key ? t('common.creating') : t(tmpl.labelKey)}
        </Button>
      ))}
    </div>
  );
}
