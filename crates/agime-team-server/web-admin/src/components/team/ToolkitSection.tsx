import { useTranslation } from 'react-i18next';
import { Tabs, TabsList, TabsTrigger, TabsContent } from '../ui/tabs';
import { SkillsTab } from './SkillsTab';
import { RecipesTab } from './RecipesTab';
import { ExtensionsTab } from './ExtensionsTab';

interface ToolkitSectionProps {
  teamId: string;
  canManage: boolean;
}

export function ToolkitSection({ teamId, canManage }: ToolkitSectionProps) {
  const { t } = useTranslation();

  return (
    <Tabs defaultValue="skills">
      <TabsList>
        <TabsTrigger value="skills">{t('teams.tabs.skills')}</TabsTrigger>
        <TabsTrigger value="recipes">{t('teams.tabs.recipes')}</TabsTrigger>
        <TabsTrigger value="extensions">{t('teams.tabs.extensions')}</TabsTrigger>
      </TabsList>
      <TabsContent value="skills">
        <SkillsTab teamId={teamId} canManage={canManage} />
      </TabsContent>
      <TabsContent value="recipes">
        <RecipesTab teamId={teamId} canManage={canManage} />
      </TabsContent>
      <TabsContent value="extensions">
        <ExtensionsTab teamId={teamId} canManage={canManage} />
      </TabsContent>
    </Tabs>
  );
}
