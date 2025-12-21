import { useTranslation } from 'react-i18next';
import { ModeSection } from '../mode/ModeSection';
import { ToolSelectionStrategySection } from '../tool_selection_strategy/ToolSelectionStrategySection';
import DictationSection from '../dictation/DictationSection';
import { SecurityToggle } from '../security/SecurityToggle';
import { ResponseStylesSection } from '../response_styles/ResponseStylesSection';
import { GoosehintsSection } from './GoosehintsSection';
import { PromptsSection } from '../prompts';
import { ThinkingModeToggle } from '../thinking/ThinkingModeToggle';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '../../ui/card';

export default function ChatSettingsSection() {
  const { t } = useTranslation('settings');
  return (
    <div className="space-y-4 pr-4 pb-8 mt-1">
      <Card className="pb-2 rounded-lg">
        <CardHeader className="pb-0">
          <CardTitle className="">{t('chat.modeTitle')}</CardTitle>
          <CardDescription>{t('chat.modeDescription')}</CardDescription>
        </CardHeader>
        <CardContent className="px-2">
          <ModeSection />
        </CardContent>
      </Card>

      <Card className="pb-2 rounded-lg">
        <CardContent className="px-2">
          <ThinkingModeToggle />
        </CardContent>
      </Card>

      <Card className="pb-2 rounded-lg">
        <CardContent className="px-2">
          <SecurityToggle />
        </CardContent>
      </Card>

      <Card className="pb-2 rounded-lg">
        <CardHeader className="pb-0">
          <CardTitle className="">{t('chat.responseStylesTitle')}</CardTitle>
          <CardDescription>{t('chat.responseStylesDescription')}</CardDescription>
        </CardHeader>
        <CardContent className="px-2">
          <ResponseStylesSection />
        </CardContent>
      </Card>

      <Card className="pb-2 rounded-lg">
        <CardContent className="px-2">
          <DictationSection />
        </CardContent>
      </Card>

      <Card className="pb-2 rounded-lg">
        <CardContent className="px-2">
          <GoosehintsSection />
        </CardContent>
      </Card>

      <Card className="pb-2 rounded-lg">
        <CardContent className="px-2">
          <PromptsSection />
        </CardContent>
      </Card>

      <Card className="pb-2 rounded-lg">
        <CardHeader className="pb-0">
          <CardTitle className="">{t('chat.toolSelectionTitle')}</CardTitle>
          <CardDescription>{t('chat.toolSelectionDescription')}</CardDescription>
        </CardHeader>
        <CardContent className="px-2">
          <ToolSelectionStrategySection />
        </CardContent>
      </Card>
    </div>
  );
}
