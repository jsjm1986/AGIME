import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { Upload, Sparkles, Book, Puzzle, Loader2, Package, FileText, Shield, FolderOpen } from 'lucide-react';
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from '../ui/dialog';
import { Button } from '../ui/button';
import { Input } from '../ui/input';
import { Textarea } from '../ui/textarea';
import { shareSkill, shareRecipe, shareExtension, uploadSkillPackage, listLocalSkills, LocalSkill } from './api';
import { useConfig, FixedExtensionEntry } from '../ConfigContext';
import { listSavedRecipes } from '../../recipe/recipe_management';
import { SkillPackageUploader } from './skill-package';
import { ProtectionLevel } from './types';

// Types
interface ShareDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  teamId: string;
  resourceType: 'skill' | 'recipe' | 'extension';
  onSuccess: () => void;
}

interface LocalRecipe {
  id: string;
  name: string;
  description?: string;
  contentYaml: string;
}

// Main component
export function ShareResourceDialog({
  open,
  onOpenChange,
  teamId,
  resourceType,
  onSuccess,
}: ShareDialogProps) {
  const { t } = useTranslation('team');

  const getTitle = () => {
    switch (resourceType) {
      case 'skill':
        return t('share.skillTitle', '分享技能到团队');
      case 'recipe':
        return t('share.recipeTitle', '分享预设任务到团队');
      case 'extension':
        return t('share.extensionTitle', '分享扩展到团队');
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            {resourceType === 'skill' && <Sparkles size={20} />}
            {resourceType === 'recipe' && <Book size={20} />}
            {resourceType === 'extension' && <Puzzle size={20} />}
            {getTitle()}
          </DialogTitle>
        </DialogHeader>

        {resourceType === 'skill' && (
          <ShareSkillForm
            teamId={teamId}
            onSuccess={() => {
              onSuccess();
              onOpenChange(false);
            }}
            onCancel={() => onOpenChange(false)}
          />
        )}

        {resourceType === 'recipe' && (
          <ShareRecipeForm
            teamId={teamId}
            onSuccess={() => {
              onSuccess();
              onOpenChange(false);
            }}
            onCancel={() => onOpenChange(false)}
          />
        )}

        {resourceType === 'extension' && (
          <ShareExtensionForm
            teamId={teamId}
            onSuccess={() => {
              onSuccess();
              onOpenChange(false);
            }}
            onCancel={() => onOpenChange(false)}
          />
        )}
      </DialogContent>
    </Dialog>
  );
}

export default ShareResourceDialog;

// Skill share form - supports local, inline and package modes
interface FormProps {
  teamId: string;
  onSuccess: () => void;
  onCancel: () => void;
}

interface ValidationResult {
  valid: boolean;
  errors: string[];
  warnings: string[];
  parsed?: {
    name: string;
    description: string;
    fileCount: number;
    totalSize: number;
  };
}

// Share mode type: local skills, custom content, or upload package
type ShareMode = 'local' | 'inline' | 'package';

function ShareSkillForm({ teamId, onSuccess, onCancel }: FormProps) {
  const { t } = useTranslation('team');
  const [mode, setMode] = useState<ShareMode>('local');

  // Local skills state
  const [localSkills, setLocalSkills] = useState<LocalSkill[]>([]);
  const [selectedLocalSkill, setSelectedLocalSkill] = useState<LocalSkill | null>(null);
  const [isLoadingLocal, setIsLoadingLocal] = useState(true);

  // Inline mode state
  const [name, setName] = useState('');
  const [content, setContent] = useState('');

  // Package mode state
  const [selectedFile, setSelectedFile] = useState<File | null>(null);
  const [validationResult, setValidationResult] = useState<ValidationResult | null>(null);

  // Common state
  const [protectionLevel, setProtectionLevel] = useState<ProtectionLevel>('team_installable');
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Load local skills on mount
  useEffect(() => {
    loadLocalSkills();
  }, []);

  const loadLocalSkills = async () => {
    setIsLoadingLocal(true);
    try {
      const skills = await listLocalSkills();
      setLocalSkills(skills);
    } catch (err) {
      console.error('Failed to load local skills:', err);
    } finally {
      setIsLoadingLocal(false);
    }
  };

  const handleFileSelected = (file: File, validation: ValidationResult) => {
    setSelectedFile(file);
    setValidationResult(validation);
    setError(null);
  };

  const handleClearFile = () => {
    setSelectedFile(null);
    setValidationResult(null);
  };

  const handleSubmit = async () => {
    setError(null);

    // Validation before setting isSubmitting
    if (mode === 'local' && !selectedLocalSkill) {
      setError(t('share.selectSkill', '请选择一个技能'));
      return;
    }
    if (mode === 'inline' && (!name.trim() || !content.trim())) {
      setError(t('share.requiredFields', '请填写名称和内容'));
      return;
    }
    if (mode === 'package' && (!selectedFile || !validationResult?.valid)) {
      setError(t('skillPackage.selectValidPackage', '请选择有效的技能包'));
      return;
    }

    setIsSubmitting(true);

    try {
      if (mode === 'local') {
        // Share based on storage type
        if (selectedLocalSkill!.storageType === 'package') {
          await shareSkill({
            teamId,
            name: selectedLocalSkill!.name,
            storageType: 'package',
            skillMd: selectedLocalSkill!.skillMd,
            files: selectedLocalSkill!.files,
            description: selectedLocalSkill!.description,
            visibility: 'team',
            protectionLevel,
          });
        } else {
          await shareSkill({
            teamId,
            name: selectedLocalSkill!.name,
            content: selectedLocalSkill!.content || selectedLocalSkill!.skillMd,
            storageType: 'inline',
            description: selectedLocalSkill!.description,
            visibility: 'team',
            protectionLevel,
          });
        }
        onSuccess();
      } else if (mode === 'inline') {
        await shareSkill({
          teamId,
          name: name.trim(),
          content: content.trim(),
          storageType: 'inline',
          visibility: 'team',
          protectionLevel,
        });
        onSuccess();
      } else {
        await uploadSkillPackage(teamId, selectedFile!, {
          visibility: 'team',
          protectionLevel,
        });
        onSuccess();
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : t('share.error', '分享失败'));
    } finally {
      setIsSubmitting(false);
    }
  };

  const canSubmit =
    mode === 'local' ? selectedLocalSkill !== null :
    mode === 'inline' ? name.trim() && content.trim() :
    selectedFile && validationResult?.valid;

  return (
    <div className="flex flex-col gap-4">
      {/* Mode selector */}
      <div>
        <label className="text-sm font-medium text-text-default mb-2 block">
          {t('share.shareMode', '分享方式')}
        </label>
        <div className="flex gap-2">
          <Button
            type="button"
            variant={mode === 'local' ? 'default' : 'outline'}
            size="sm"
            onClick={() => setMode('local')}
            className="flex-1"
          >
            <FolderOpen className="h-4 w-4 mr-1.5" />
            {t('share.localSkill', '本地技能')}
          </Button>
          <Button
            type="button"
            variant={mode === 'inline' ? 'default' : 'outline'}
            size="sm"
            onClick={() => setMode('inline')}
            className="flex-1"
          >
            <FileText className="h-4 w-4 mr-1.5" />
            {t('share.customContent', '自定义')}
          </Button>
          <Button
            type="button"
            variant={mode === 'package' ? 'default' : 'outline'}
            size="sm"
            onClick={() => setMode('package')}
            className="flex-1"
          >
            <Package className="h-4 w-4 mr-1.5" />
            {t('share.uploadPackage', '上传包')}
          </Button>
        </div>
        <p className="text-xs text-text-muted mt-1.5">
          {mode === 'local'
            ? t('share.localSkillDesc', '选择已安装的本地技能进行分享')
            : mode === 'inline'
            ? t('skillPackage.inlineDesc', '输入简单的提示词文本')
            : t('skillPackage.packageDesc', '上传包含 SKILL.md 和附加文件的 ZIP 包')}
        </p>
      </div>

      {/* Local skills mode */}
      {mode === 'local' && (
        <LocalSkillSelector
          skills={localSkills}
          selectedSkill={selectedLocalSkill}
          onSelect={setSelectedLocalSkill}
          isLoading={isLoadingLocal}
        />
      )}

      {/* Inline mode: name + content */}
      {mode === 'inline' && (
        <>
          <div>
            <label className="text-sm font-medium text-text-default mb-1.5 block">
              {t('share.name', '名称')} *
            </label>
            <Input
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder={t('share.skillNamePlaceholder', '例如：代码审查专家')}
            />
          </div>
          <div>
            <label className="text-sm font-medium text-text-default mb-1.5 block">
              {t('share.content', '内容')} *
            </label>
            <Textarea
              value={content}
              onChange={(e) => setContent(e.target.value)}
              placeholder={t('share.skillContentPlaceholder', '输入技能提示词内容...')}
              rows={6}
              className="resize-none"
            />
          </div>
        </>
      )}

      {/* Package mode: ZIP upload */}
      {mode === 'package' && (
        <>
          <div>
            <label className="text-sm font-medium text-text-default mb-1.5 block">
              {t('skillPackage.uploadPackage', '上传技能包')} *
            </label>
            <SkillPackageUploader
              onFileSelected={handleFileSelected}
              onClear={handleClearFile}
              selectedFile={selectedFile}
              validationResult={validationResult}
              disabled={isSubmitting}
            />
          </div>
          <div className="text-xs text-text-muted bg-background-muted p-3 rounded-lg">
            <p className="font-medium mb-1">{t('skillPackage.formatHelp', '包格式说明')}:</p>
            <ul className="list-disc list-inside space-y-0.5">
              <li>{t('skillPackage.formatSkillMd', 'SKILL.md - 必需，包含 YAML 头部和技能指令')}</li>
              <li>{t('skillPackage.formatScripts', 'scripts/ - 可选，可执行脚本文件')}</li>
              <li>{t('skillPackage.formatReferences', 'references/ - 可选，参考文档')}</li>
              <li>{t('skillPackage.formatAssets', 'assets/ - 可选，模板和资源文件')}</li>
            </ul>
          </div>
        </>
      )}

      {/* Protection Level Selector */}
      <ProtectionLevelSelector
        value={protectionLevel}
        onChange={setProtectionLevel}
      />

      {error && (
        <p className="text-sm text-red-500">{error}</p>
      )}

      <div className="flex justify-end gap-3 pt-2">
        <Button variant="outline" onClick={onCancel} disabled={isSubmitting}>
          {t('share.cancel', '取消')}
        </Button>
        <Button onClick={handleSubmit} disabled={isSubmitting || !canSubmit}>
          {isSubmitting && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
          {t('share.submit', '分享')}
        </Button>
      </div>
    </div>
  );
}

// Recipe share form - select from local recipes
function ShareRecipeForm({ teamId, onSuccess, onCancel }: FormProps) {
  const { t } = useTranslation('team');
  const [recipes, setRecipes] = useState<LocalRecipe[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [selectedRecipe, setSelectedRecipe] = useState<LocalRecipe | null>(null);
  const [protectionLevel, setProtectionLevel] = useState<ProtectionLevel>('team_installable');
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    loadRecipes();
  }, []);

  const loadRecipes = async () => {
    try {
      const manifests = await listSavedRecipes();
      const localRecipes: LocalRecipe[] = manifests.map((m: any) => ({
        id: m.id || m.recipe?.title || 'unknown',
        name: m.recipe?.title || 'Untitled',
        description: m.recipe?.description,
        contentYaml: JSON.stringify(m.recipe, null, 2),
      }));
      setRecipes(localRecipes);
    } catch (err) {
      console.error('Failed to load recipes:', err);
    } finally {
      setIsLoading(false);
    }
  };

  const handleSubmit = async () => {
    if (!selectedRecipe) return;

    setIsSubmitting(true);
    setError(null);

    try {
      await shareRecipe({
        teamId,
        name: selectedRecipe.name,
        contentYaml: selectedRecipe.contentYaml,
        description: selectedRecipe.description,
        visibility: 'team',
        protectionLevel,
      });
      onSuccess();
    } catch (err) {
      setError(err instanceof Error ? err.message : t('share.error', '分享失败'));
    } finally {
      setIsSubmitting(false);
    }
  };

  if (isLoading) {
    return (
      <div className="flex items-center justify-center py-8">
        <Loader2 className="h-6 w-6 animate-spin text-teal-500" />
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-4">
      {recipes.length === 0 ? (
        <p className="text-center text-text-muted py-8">
          {t('share.noLocalRecipes', '没有本地预设任务')}
        </p>
      ) : (
        <>
          <div>
            <label className="text-sm font-medium text-text-default mb-1.5 block">
              {t('share.selectRecipe', '选择预设任务')} *
            </label>
            <div className="max-h-[200px] overflow-y-auto space-y-2">
              {recipes.map((recipe) => (
                <label
                  key={recipe.id}
                  className={`flex items-center gap-3 p-3 rounded-lg border cursor-pointer transition-colors ${
                    selectedRecipe?.id === recipe.id
                      ? 'border-teal-500 bg-teal-50 dark:bg-teal-900/20'
                      : 'border-border-default hover:border-border-hover'
                  }`}
                >
                  <input
                    type="radio"
                    name="recipe"
                    checked={selectedRecipe?.id === recipe.id}
                    onChange={() => setSelectedRecipe(recipe)}
                    className="shrink-0"
                  />
                  <div className="flex-1 min-w-0">
                    <p className="font-medium text-text-default truncate">{recipe.name}</p>
                    {recipe.description && (
                      <p className="text-xs text-text-muted truncate">{recipe.description}</p>
                    )}
                  </div>
                </label>
              ))}
            </div>
          </div>

          {/* Protection Level Selector */}
          <ProtectionLevelSelector
            value={protectionLevel}
            onChange={setProtectionLevel}
          />
        </>
      )}

      {error && <p className="text-sm text-red-500">{error}</p>}

      <div className="flex justify-end gap-3 pt-2">
        <Button variant="outline" onClick={onCancel} disabled={isSubmitting}>
          {t('share.cancel', '取消')}
        </Button>
        <Button onClick={handleSubmit} disabled={isSubmitting || !selectedRecipe}>
          {isSubmitting && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
          {t('share.submit', '分享')}
        </Button>
      </div>
    </div>
  );
}

// Extension share form - select from local extensions
function ShareExtensionForm({ teamId, onSuccess, onCancel }: FormProps) {
  const { t } = useTranslation('team');
  const { extensionsList } = useConfig();
  const [sharingName, setSharingName] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  // 只允许分享非内置扩展（bundled 字段标识内置扩展）
  const shareableExtensions = extensionsList?.filter(
    (ext) => ext.enabled && !('bundled' in ext && ext.bundled)
  ) || [];

  const handleShare = async (ext: FixedExtensionEntry) => {
    setSharingName(ext.name);
    setError(null);

    try {
      // Extract config by removing enabled flag
      const { enabled, ...config } = ext;
      await shareExtension({
        teamId,
        name: ext.name,
        extensionType: ext.type,
        config: config as Record<string, unknown>,
        visibility: 'team',
      });
      onSuccess();
    } catch (err) {
      setError(err instanceof Error ? err.message : t('share.error', '分享失败'));
      setSharingName(null);
    }
  };

  return (
    <div className="flex flex-col gap-3">
      {shareableExtensions.length === 0 ? (
        <p className="text-center text-text-muted py-8">
          {t('share.noLocalExtensions', '没有可分享的本地扩展')}
        </p>
      ) : (
        <div className="max-h-[300px] overflow-y-auto space-y-2">
          {shareableExtensions.map((ext) => (
            <ExtensionCard
              key={ext.name}
              extension={ext}
              onShare={() => handleShare(ext)}
              isSharing={sharingName === ext.name}
            />
          ))}
        </div>
      )}

      {error && <p className="text-sm text-red-500">{error}</p>}

      <div className="flex justify-end pt-2">
        <Button variant="outline" onClick={onCancel}>
          {t('share.cancel', '取消')}
        </Button>
      </div>
    </div>
  );
}

// Extension card component
function ExtensionCard({
  extension,
  onShare,
  isSharing,
}: {
  extension: FixedExtensionEntry;
  onShare: () => void;
  isSharing: boolean;
}) {
  const { t } = useTranslation('team');

  return (
    <div className="flex items-center justify-between p-3 bg-background-muted rounded-lg">
      <div className="flex-1 min-w-0">
        <p className="font-medium text-text-default truncate">{extension.name}</p>
        <p className="text-xs text-text-muted">{extension.type}</p>
      </div>
      <Button
        size="sm"
        variant="outline"
        onClick={onShare}
        disabled={isSharing}
        className="ml-3 shrink-0"
      >
        {isSharing ? (
          <Loader2 className="h-4 w-4 animate-spin" />
        ) : (
          <Upload className="h-4 w-4" />
        )}
        <span className="ml-1">{t('share.submit', '分享')}</span>
      </Button>
    </div>
  );
}

// Local skill selector component
function LocalSkillSelector({
  skills,
  selectedSkill,
  onSelect,
  isLoading,
}: {
  skills: LocalSkill[];
  selectedSkill: LocalSkill | null;
  onSelect: (skill: LocalSkill | null) => void;
  isLoading: boolean;
}) {
  const { t } = useTranslation('team');

  if (isLoading) {
    return (
      <div className="flex items-center justify-center py-8">
        <Loader2 className="h-6 w-6 animate-spin text-teal-500" />
      </div>
    );
  }

  if (skills.length === 0) {
    return (
      <p className="text-center text-text-muted py-8">
        {t('share.noLocalSkills', '没有本地技能')}
      </p>
    );
  }

  return (
    <div>
      <label className="text-sm font-medium text-text-default mb-1.5 block">
        {t('share.selectSkill', '选择技能')} *
      </label>
      <div className="max-h-[200px] overflow-y-auto space-y-2">
        {skills.map((skill) => (
          <label
            key={skill.path}
            className={`flex items-center gap-3 p-3 rounded-lg border cursor-pointer transition-colors ${
              selectedSkill?.name === skill.name
                ? 'border-teal-500 bg-teal-50 dark:bg-teal-900/20'
                : 'border-border-default hover:border-border-hover'
            }`}
          >
            <input
              type="radio"
              name="localSkill"
              checked={selectedSkill?.name === skill.name}
              onChange={() => onSelect(skill)}
              className="shrink-0"
            />
            <div className="flex-1 min-w-0">
              <div className="flex items-center gap-2">
                <p className="font-medium text-text-default truncate">{skill.name}</p>
                <span className="text-xs px-1.5 py-0.5 rounded bg-background-muted text-text-muted">
                  {skill.storageType === 'package' ? t('share.packageType', '包') : t('share.inlineType', '简单')}
                </span>
              </div>
              {skill.description && (
                <p className="text-xs text-text-muted truncate">{skill.description}</p>
              )}
            </div>
          </label>
        ))}
      </div>
    </div>
  );
}

// Protection Level Selector component
function ProtectionLevelSelector({
  value,
  onChange,
}: {
  value: ProtectionLevel;
  onChange: (level: ProtectionLevel) => void;
}) {
  const { t } = useTranslation('team');

  const levels: { value: ProtectionLevel; label: string; description: string; icon: string }[] = [
    {
      value: 'public',
      label: t('protectionLevel.public', '公开'),
      description: t('protectionLevel.publicDesc', '任何人可访问和安装'),
      icon: '🌐',
    },
    {
      value: 'team_installable',
      label: t('protectionLevel.teamInstallable', '团队可安装'),
      description: t('protectionLevel.teamInstallableDesc', '团队成员可安装到本地'),
      icon: '👥',
    },
    {
      value: 'team_online_only',
      label: t('protectionLevel.teamOnlineOnly', '仅在线使用'),
      description: t('protectionLevel.teamOnlineOnlyDesc', '不允许本地安装，仅在线访问'),
      icon: '☁️',
    },
    {
      value: 'controlled',
      label: t('protectionLevel.controlled', '受控访问'),
      description: t('protectionLevel.controlledDesc', '完整审计，高度机密内容'),
      icon: '🔒',
    },
  ];

  return (
    <div>
      <label className="text-sm font-medium text-text-default mb-1.5 flex items-center gap-1.5">
        <Shield size={14} />
        {t('share.protectionLevel', '保护级别')}
      </label>
      <div className="space-y-2">
        {levels.map((level) => (
          <label
            key={level.value}
            className={`flex items-start gap-3 p-3 rounded-lg border cursor-pointer transition-colors ${
              value === level.value
                ? 'border-teal-500 bg-teal-50 dark:bg-teal-900/20'
                : 'border-border-default hover:border-border-hover'
            }`}
          >
            <input
              type="radio"
              name="protectionLevel"
              value={level.value}
              checked={value === level.value}
              onChange={() => onChange(level.value)}
              className="mt-1"
            />
            <div className="flex-1">
              <div className="flex items-center gap-2">
                <span>{level.icon}</span>
                <span className="font-medium text-text-default">{level.label}</span>
              </div>
              <p className="text-xs text-text-muted mt-0.5">{level.description}</p>
            </div>
          </label>
        ))}
      </div>
    </div>
  );
}
