import React, { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { FileText, Eye, EyeOff } from 'lucide-react';
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
  const [previewingTemplate, setPreviewingTemplate] = useState<string | null>(null);

  const togglePreview = (templateName: string) => {
    setPreviewingTemplate(previewingTemplate === templateName ? null : templateName);
  };

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
            className="rounded-lg border border-border-subtle bg-background-secondary hover:border-border-active transition-colors"
          >
            <div className="flex items-start gap-4 p-4">
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
              <div className="flex gap-2">
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => togglePreview(template.name)}
                  title={previewingTemplate === template.name ? t('prompts.hidePreview') : t('prompts.preview')}
                >
                  {previewingTemplate === template.name ? (
                    <EyeOff className="h-4 w-4" />
                  ) : (
                    <Eye className="h-4 w-4" />
                  )}
                </Button>
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => onSelect(template)}
                >
                  {t('prompts.useTemplate')}
                </Button>
              </div>
            </div>
            {previewingTemplate === template.name && (
              <div className="border-t border-border-subtle p-4">
                <pre className="text-xs text-text-muted whitespace-pre-wrap max-h-64 overflow-auto bg-background-primary rounded p-3 font-mono">
                  {template.content}
                </pre>
              </div>
            )}
          </div>
        ))}
      </div>
    </div>
  );
};
