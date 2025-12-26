import { useTranslation } from 'react-i18next';
import { ModeSection } from '../mode/ModeSection';
import { ToolSelectionStrategySection } from '../tool_selection_strategy/ToolSelectionStrategySection';
import DictationSection from '../dictation/DictationSection';
import { SecurityToggle } from '../security/SecurityToggle';
import { ResponseStylesSection } from '../response_styles/ResponseStylesSection';
import { AgimehintsSection } from './AgimehintsSection';
import { PromptsSection } from '../prompts';
import { ThinkingModeToggle } from '../thinking/ThinkingModeToggle';
import { SettingsCard } from '../common';
import { FileText, Sparkles, Sliders, MessageSquare, Wrench } from 'lucide-react';

export default function ChatSettingsSection() {
  const { t } = useTranslation('settings');
  return (
    <div className="space-y-6 pb-8 mt-1">
      {/* 对话模式 */}
      <SettingsCard
        icon={<Sliders className="h-5 w-5" />}
        title={t('chat.modeTitle')}
        description={t('chat.modeDescription')}
      >
        <ModeSection />
      </SettingsCard>

      {/* AI 能力 - 合并扩展思考和安全检测 */}
      <SettingsCard
        icon={<Sparkles className="h-5 w-5" />}
        title={t('chat.aiCapabilitiesTitle', 'AI 能力')}
        description={t('chat.aiCapabilitiesDescription', '配置 AI 的高级功能')}
      >
        <ThinkingModeToggle asInline />
        <SecurityToggle asInline />
      </SettingsCard>

      {/* 响应风格 */}
      <SettingsCard
        icon={<MessageSquare className="h-5 w-5" />}
        title={t('chat.responseStylesTitle')}
        description={t('chat.responseStylesDescription')}
      >
        <ResponseStylesSection />
      </SettingsCard>

      {/* 输入与提示 - 合并语音输入、项目提示、系统提示词 */}
      <SettingsCard
        icon={<FileText className="h-5 w-5" />}
        title={t('chat.inputAndPromptsTitle', '输入与提示')}
        description={t('chat.inputAndPromptsDescription', '配置输入方式和系统提示词')}
      >
        <DictationSection />
        <AgimehintsSection />
        <PromptsSection />
      </SettingsCard>

      {/* 工具选择策略 */}
      <SettingsCard
        icon={<Wrench className="h-5 w-5" />}
        title={t('chat.toolSelectionTitle')}
        description={t('chat.toolSelectionDescription')}
      >
        <ToolSelectionStrategySection />
      </SettingsCard>
    </div>
  );
}
