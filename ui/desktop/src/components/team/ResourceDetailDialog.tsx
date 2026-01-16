import React, { useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from '../ui/dialog';
import { Button } from '../ui/button';
import {
  Check,
  AlertTriangle,
  Calendar,
  User,
  Tag,
  Eye,
  Shield,
  FileText,
  Settings,
  Package,
  Download,
  Loader2,
} from 'lucide-react';
import { SharedSkill, SharedRecipe, SharedExtension, isPackageSkill, getSkillContent, formatPackageSize, ProtectionLevel, allowsLocalInstall } from './types';
import { FileTreeView } from './skill-package';
import { downloadSkillPackage } from './api';

type ResourceType = 'skill' | 'recipe' | 'extension';
type Resource = SharedSkill | SharedRecipe | SharedExtension;

interface ResourceDetailDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  resourceType: ResourceType;
  resource: Resource | null;
  isLoading?: boolean;
  error?: string | null;
}

const ResourceDetailDialog: React.FC<ResourceDetailDialogProps> = ({
  open,
  onOpenChange,
  resourceType,
  resource,
  isLoading,
  error,
}) => {
  const { t } = useTranslation('team');
  const [isExporting, setIsExporting] = useState(false);

  const formatDate = (dateString: string) => {
    return new Date(dateString).toLocaleString();
  };

  const getTitle = () => {
    switch (resourceType) {
      case 'skill':
        return t('skills');
      case 'recipe':
        return t('recipes');
      case 'extension':
        return t('extensions');
    }
  };

  const renderLoading = () => (
    <div className="flex items-center justify-center py-12">
      <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-teal-500"></div>
    </div>
  );

  const renderError = () => (
    <div className="flex flex-col items-center justify-center py-12">
      <AlertTriangle className="h-8 w-8 text-red-500 mb-2" />
      <p className="text-red-500">{error || t('manage.loadDetailError')}</p>
    </div>
  );

  const renderDetailRow = (
    icon: React.ReactNode,
    label: string,
    value: React.ReactNode
  ) => (
    <div className="flex items-start gap-3 py-3 border-b border-border-subtle last:border-b-0">
      <div className="text-text-muted mt-0.5">{icon}</div>
      <div className="flex-1">
        <p className="text-xs text-text-muted mb-1">{label}</p>
        <div className="text-sm text-text-default">{value}</div>
      </div>
    </div>
  );

  const renderTags = (tags: string[]) => {
    if (!tags || tags.length === 0) {
      return <span className="text-text-muted">{t('detail.noTags')}</span>;
    }
    return (
      <div className="flex flex-wrap gap-1.5">
        {tags.map((tag) => (
          <span
            key={tag}
            className="text-xs bg-background-muted px-2 py-1 rounded"
          >
            {tag}
          </span>
        ))}
      </div>
    );
  };

  const renderProtectionLevel = (level: ProtectionLevel) => {
    const levelConfig: Record<ProtectionLevel, { label: string; icon: string; color: string }> = {
      public: {
        label: t('protectionLevel.public', '公开'),
        icon: '🌐',
        color: 'text-green-600 dark:text-green-400',
      },
      team_installable: {
        label: t('protectionLevel.teamInstallable', '团队可安装'),
        icon: '👥',
        color: 'text-blue-600 dark:text-blue-400',
      },
      team_online_only: {
        label: t('protectionLevel.teamOnlineOnly', '仅在线使用'),
        icon: '☁️',
        color: 'text-orange-600 dark:text-orange-400',
      },
      controlled: {
        label: t('protectionLevel.controlled', '受控访问'),
        icon: '🔒',
        color: 'text-red-600 dark:text-red-400',
      },
    };

    const config = levelConfig[level] || levelConfig.team_installable;

    return (
      <div className="flex items-center gap-2">
        <span>{config.icon}</span>
        <span className={config.color}>{config.label}</span>
        {!allowsLocalInstall(level) && (
          <span className="text-xs text-text-muted">
            ({t('protectionLevel.noLocalInstall', '不可本地安装')})
          </span>
        )}
      </div>
    );
  };

  const renderSkillDetail = (skill: SharedSkill) => {
    const isPackage = isPackageSkill(skill);
    const content = getSkillContent(skill);

    const handleExport = async () => {
      setIsExporting(true);
      try {
        const blob = await downloadSkillPackage(skill.id);
        const url = URL.createObjectURL(blob);
        const a = document.createElement('a');
        a.href = url;
        a.download = `${skill.name}.zip`;
        document.body.appendChild(a);
        a.click();
        document.body.removeChild(a);
        URL.revokeObjectURL(url);
      } catch (err) {
        console.error('Export failed:', err);
      } finally {
        setIsExporting(false);
      }
    };

    return (
      <div className="space-y-1">
        {/* Header with export button for package skills */}
        {isPackage && (
          <div className="flex items-center justify-between py-3 border-b border-border-subtle">
            <div className="flex items-center gap-2">
              <Package size={16} className="text-teal-500" />
              <span className="text-sm font-medium text-teal-600 dark:text-teal-400">
                {t('skillPackage.packageMode', '包模式')}
              </span>
              {skill.packageSize && (
                <span className="text-xs text-text-muted">
                  ({formatPackageSize(skill.packageSize)})
                </span>
              )}
            </div>
            <Button
              variant="outline"
              size="sm"
              onClick={handleExport}
              disabled={isExporting}
            >
              {isExporting ? (
                <Loader2 className="h-4 w-4 mr-1.5 animate-spin" />
              ) : (
                <Download className="h-4 w-4 mr-1.5" />
              )}
              {t('skillPackage.export', '导出 ZIP')}
            </Button>
          </div>
        )}

        {renderDetailRow(
          <FileText size={16} />,
          t('detail.name'),
          skill.name
        )}
        {renderDetailRow(
          <FileText size={16} />,
          t('detail.description'),
          skill.description || <span className="text-text-muted">{t('detail.noDescription')}</span>
        )}

        {/* Storage type indicator */}
        {renderDetailRow(
          isPackage ? <Package size={16} /> : <FileText size={16} />,
          t('skillPackage.storageType', '存储类型'),
          isPackage ? t('skillPackage.package', '包模式') : t('skillPackage.inline', '简单模式')
        )}

        {/* Content display */}
        {renderDetailRow(
          <FileText size={16} />,
          isPackage ? 'SKILL.md' : t('detail.content'),
          <pre className="whitespace-pre-wrap text-xs bg-background-muted p-3 rounded-md max-h-48 overflow-y-auto font-mono">
            {content || skill.content}
          </pre>
        )}

        {/* File tree for package skills */}
        {isPackage && skill.files && skill.files.length > 0 && (
          <div className="py-3 border-b border-border-subtle">
            <p className="text-xs text-text-muted mb-2">{t('skillPackage.files', '附加文件')}</p>
            <FileTreeView
              files={skill.files}
              showActions={false}
              compact={true}
            />
          </div>
        )}

        {/* Metadata for package skills */}
        {isPackage && skill.metadata && (
          <>
            {skill.metadata.author && renderDetailRow(
              <User size={16} />,
              t('skillPackage.metaAuthor', '包作者'),
              skill.metadata.author
            )}
            {skill.metadata.license && renderDetailRow(
              <FileText size={16} />,
              t('skillPackage.license', '许可证'),
              skill.metadata.license
            )}
            {skill.metadata.homepage && renderDetailRow(
              <FileText size={16} />,
              t('skillPackage.homepage', '主页'),
              <a href={skill.metadata.homepage} target="_blank" rel="noopener noreferrer" className="text-teal-500 hover:underline">
                {skill.metadata.homepage}
              </a>
            )}
            {skill.metadata.keywords && skill.metadata.keywords.length > 0 && renderDetailRow(
              <Tag size={16} />,
              t('skillPackage.keywords', '关键词'),
              renderTags(skill.metadata.keywords)
            )}
          </>
        )}

        {renderDetailRow(
          <Tag size={16} />,
          t('detail.tags'),
          renderTags(skill.tags)
        )}
        {renderDetailRow(
          <Eye size={16} />,
          t('detail.visibility'),
          skill.visibility === 'public' ? t('edit.visibilityPublic') : t('edit.visibilityTeam')
        )}
        {renderDetailRow(
          <Shield size={16} />,
          t('detail.protectionLevel'),
          renderProtectionLevel(skill.protectionLevel)
        )}
        {renderDetailRow(
          <FileText size={16} />,
          t('detail.version'),
          `v${skill.version}`
        )}
        {renderDetailRow(
          <User size={16} />,
          t('detail.author'),
          skill.authorId
        )}
        {renderDetailRow(
          <Calendar size={16} />,
          t('detail.createdAt'),
          formatDate(skill.createdAt)
        )}
        {renderDetailRow(
          <Calendar size={16} />,
          t('detail.updatedAt'),
          formatDate(skill.updatedAt)
        )}
      </div>
    );
  };

  const renderRecipeDetail = (recipe: SharedRecipe) => (
    <div className="space-y-1">
      {renderDetailRow(
        <FileText size={16} />,
        t('detail.name'),
        recipe.name
      )}
      {renderDetailRow(
        <FileText size={16} />,
        t('detail.description'),
        recipe.description || <span className="text-text-muted">{t('detail.noDescription')}</span>
      )}
      {renderDetailRow(
        <FileText size={16} />,
        t('detail.content'),
        <pre className="whitespace-pre-wrap text-xs bg-background-muted p-3 rounded-md max-h-48 overflow-y-auto font-mono">
          {recipe.contentYaml}
        </pre>
      )}
      {recipe.category && renderDetailRow(
        <Tag size={16} />,
        t('detail.category'),
        recipe.category
      )}
      {renderDetailRow(
        <Tag size={16} />,
        t('detail.tags'),
        renderTags(recipe.tags)
      )}
      {renderDetailRow(
        <Eye size={16} />,
        t('detail.visibility'),
        recipe.visibility === 'public' ? t('edit.visibilityPublic') : t('edit.visibilityTeam')
      )}
      {renderDetailRow(
        <Shield size={16} />,
        t('detail.protectionLevel'),
        renderProtectionLevel(recipe.protectionLevel)
      )}
      {renderDetailRow(
        <FileText size={16} />,
        t('detail.version'),
        `v${recipe.version}`
      )}
      {renderDetailRow(
        <User size={16} />,
        t('detail.author'),
        recipe.authorId
      )}
      {renderDetailRow(
        <Calendar size={16} />,
        t('detail.createdAt'),
        formatDate(recipe.createdAt)
      )}
      {renderDetailRow(
        <Calendar size={16} />,
        t('detail.updatedAt'),
        formatDate(recipe.updatedAt)
      )}
    </div>
  );

  const renderExtensionDetail = (extension: SharedExtension) => (
    <div className="space-y-1">
      {renderDetailRow(
        <FileText size={16} />,
        t('detail.name'),
        extension.name
      )}
      {renderDetailRow(
        <FileText size={16} />,
        t('detail.description'),
        extension.description || <span className="text-text-muted">{t('detail.noDescription')}</span>
      )}
      {renderDetailRow(
        <Settings size={16} />,
        t('detail.type'),
        <span className="capitalize">{extension.extensionType}</span>
      )}
      {renderDetailRow(
        <Settings size={16} />,
        t('detail.config'),
        <pre className="whitespace-pre-wrap text-xs bg-background-muted p-3 rounded-md max-h-48 overflow-y-auto font-mono">
          {JSON.stringify(extension.config, null, 2)}
        </pre>
      )}
      {renderDetailRow(
        <Tag size={16} />,
        t('detail.tags'),
        renderTags(extension.tags)
      )}
      {renderDetailRow(
        <Eye size={16} />,
        t('detail.visibility'),
        extension.visibility === 'public' ? t('edit.visibilityPublic') : t('edit.visibilityTeam')
      )}
      {renderDetailRow(
        <Shield size={16} />,
        t('detail.protectionLevel'),
        renderProtectionLevel(extension.protectionLevel)
      )}
      {renderDetailRow(
        <Shield size={16} />,
        t('detail.securityStatus'),
        extension.securityReviewed ? (
          <div className="flex items-center gap-2">
            <Check size={14} className="text-green-500" />
            <span className="text-green-600 dark:text-green-400">{t('detail.reviewed')}</span>
          </div>
        ) : (
          <div className="flex items-center gap-2">
            <AlertTriangle size={14} className="text-yellow-500" />
            <span className="text-yellow-600 dark:text-yellow-400">{t('detail.notReviewed')}</span>
          </div>
        )
      )}
      {extension.securityReviewed && extension.reviewedBy && renderDetailRow(
        <User size={16} />,
        t('detail.reviewedBy'),
        extension.reviewedBy
      )}
      {extension.securityNotes && renderDetailRow(
        <FileText size={16} />,
        t('detail.reviewNotes'),
        extension.securityNotes
      )}
      {renderDetailRow(
        <FileText size={16} />,
        t('detail.version'),
        `v${extension.version}`
      )}
      {renderDetailRow(
        <User size={16} />,
        t('detail.author'),
        extension.authorId
      )}
      {renderDetailRow(
        <Calendar size={16} />,
        t('detail.createdAt'),
        formatDate(extension.createdAt)
      )}
      {renderDetailRow(
        <Calendar size={16} />,
        t('detail.updatedAt'),
        formatDate(extension.updatedAt)
      )}
    </div>
  );

  const renderContent = () => {
    if (isLoading) return renderLoading();
    if (error) return renderError();
    if (!resource) return null;

    switch (resourceType) {
      case 'skill':
        return renderSkillDetail(resource as SharedSkill);
      case 'recipe':
        return renderRecipeDetail(resource as SharedRecipe);
      case 'extension':
        return renderExtensionDetail(resource as SharedExtension);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-2xl max-h-[80vh] overflow-hidden flex flex-col">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            {getTitle()} - {t('detail.title')}
          </DialogTitle>
        </DialogHeader>
        <div className="flex-1 overflow-y-auto pr-2">
          {renderContent()}
        </div>
        <div className="flex justify-end pt-4 border-t border-border-subtle">
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            {t('detail.close')}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
};

export default ResourceDetailDialog;
