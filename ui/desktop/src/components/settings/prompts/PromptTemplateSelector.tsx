import React from 'react';
import { useTranslation } from 'react-i18next';
import { FileText } from 'lucide-react';
import { Button } from '../../ui/button';
import { PromptTemplate } from '../../../lib/api/prompts';

interface PromptTemplateSelectorProps {
  templates: PromptTemplate[];
  onSelect: (template: PromptTemplate) => void;
}

export const PromptTemplateSelector: React.FC<PromptTemplateSelectorProps> = ({
  templates,
  onSelect,
}) => {
  const { t } = useTranslation('settings');

  if (templates.length === 0) {
    return (
      <div className="flex items-center justify-center py-12 text-text-muted">
        {t('prompts.noTemplates')}
      </div>
    );
  }

  return (
    <div className="space-y-4">
      <p className="text-sm text-text-muted">
        {t('prompts.templateDescription')}
      </p>
      <div className="grid gap-4">
        {templates.map((template) => (
          <div
            key={template.name}
            className="flex items-start gap-4 p-4 rounded-lg border border-border-subtle bg-background-secondary hover:border-border-active transition-colors"
          >
            <div className="flex-shrink-0 mt-1">
              <FileText className="h-5 w-5 text-text-muted" />
            </div>
            <div className="flex-1 min-w-0">
              <h4 className="font-medium text-text-default">
                {template.display_name}
              </h4>
              <p className="text-sm text-text-muted mt-1">
                {template.description}
              </p>
              <div className="mt-2 text-xs text-text-muted">
                {template.content.length} {t('prompts.characters')}
              </div>
            </div>
            <Button
              variant="outline"
              size="sm"
              onClick={() => onSelect(template)}
            >
              {t('prompts.useTemplate')}
            </Button>
          </div>
        ))}
      </div>
    </div>
  );
};
