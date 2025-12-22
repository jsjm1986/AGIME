import { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Button } from '../../ui/button';
import { ChevronDownIcon, SlidersHorizontal } from 'lucide-react';
import { getTools, PermissionLevel, ToolInfo, upsertPermissions } from '../../../api';
import { Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle } from '../../ui/dialog';
import {
  DropdownMenu,
  DropdownMenuTrigger,
  DropdownMenuContent,
  DropdownMenuItem,
} from '../../ui/dropdown-menu';
import { useChatContext } from '../../../contexts/ChatContext';

function getFirstSentence(text: string): string {
  const match = text.match(/^([^.?!]+[.?!])/);
  return match ? match[0] : '';
}

interface PermissionModalProps {
  extensionName: string;
  onClose: () => void;
}

export default function PermissionModal({ extensionName, onClose }: PermissionModalProps) {
  const { t } = useTranslation('settings');
  const { t: tCommon } = useTranslation('common');
  const { t: tTools } = useTranslation('tools');

  // Helper function to get translated tool description
  const getToolDescription = (toolName: string, originalDescription: string): string => {
    // Try to get translation using tool name as key
    const translationKey = `${toolName}.description`;
    const translated = tTools(translationKey, { defaultValue: '' });
    // If translation exists and is not the key itself, use it; otherwise use original
    if (translated && translated !== translationKey && translated !== '') {
      return translated;
    }
    return getFirstSentence(originalDescription);
  };

  const permissionOptions = [
    { value: 'always_allow', label: t('permission.alwaysAllow') },
    { value: 'ask_before', label: t('permission.askBefore') },
    { value: 'never_allow', label: t('permission.neverAllow') },
  ] as { value: PermissionLevel; label: string }[];

  const chatContext = useChatContext();
  const sessionId = chatContext?.chat.sessionId || '';

  const [tools, setTools] = useState<ToolInfo[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [updatedPermissions, setUpdatedPermissions] = useState<Record<string, string>>({});

  const hasChanges = useMemo(() => {
    return Object.keys(updatedPermissions).some(
      (toolName) =>
        updatedPermissions[toolName] !== tools.find((tool) => tool.name === toolName)?.permission
    );
  }, [updatedPermissions, tools]);

  useEffect(() => {
    const fetchTools = async () => {
      setIsLoading(true);
      try {
        const response = await getTools({
          query: { extension_name: extensionName, session_id: sessionId },
        });
        if (response.error) {
          console.error('Failed to get tools');
        } else {
          const filteredTools = (response.data || []).filter(
            (tool: ToolInfo) =>
              tool.name !== 'platform__read_resource' && tool.name !== 'platform__list_resources'
          );
          setTools(filteredTools);
        }
      } catch (err) {
        console.error('Error fetching tools:', err);
      } finally {
        setIsLoading(false);
      }
    };

    fetchTools();
  }, [extensionName, sessionId]);

  const handleSettingChange = (toolName: string, newPermission: PermissionLevel) => {
    setUpdatedPermissions((prev) => ({
      ...prev,
      [toolName]: newPermission,
    }));
  };

  const handleClose = () => {
    onClose();
  };

  const handleSave = async () => {
    try {
      const payload = {
        tool_permissions: Object.entries(updatedPermissions).map(([toolName, permission]) => ({
          tool_name: toolName,
          permission: permission as PermissionLevel,
        })),
      };

      if (payload.tool_permissions.length === 0) {
        onClose();
        return;
      }

      const response = await upsertPermissions({
        body: payload,
      });
      if (response.error) {
        console.error('Failed to save permissions:', response.error);
      } else {
        console.log('Permissions updated successfully');
        onClose();
      }
    } catch (err) {
      console.error('Error saving permissions:', err);
    }
  };

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) {
          handleClose();
        }
      }}
    >
      <DialogContent className="sm:max-w-[500px] max-h-[90vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <SlidersHorizontal className="text-iconStandard" size={24} />
            {extensionName}
          </DialogTitle>
        </DialogHeader>

        <div className="py-4">
          {isLoading ? (
            <div className="flex items-center justify-center">
              {/* Loading spinner */}
              <svg
                className="animate-spin h-8 w-8 text-grey-50 dark:text-white"
                xmlns="http://www.w3.org/2000/svg"
                fill="none"
                viewBox="0 0 24 24"
              >
                <circle
                  className="opacity-25"
                  cx="12"
                  cy="12"
                  r="10"
                  stroke="currentColor"
                  strokeWidth="4"
                ></circle>
                <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"></path>
              </svg>
            </div>
          ) : tools.length === 0 ? (
            <div className="flex items-center justify-center py-8">
              <p className="text-sm text-textSubtle">{t('permission.noToolsAvailable')}</p>
            </div>
          ) : (
            <div className="space-y-4">
              {tools.map((tool) => (
                <div
                  key={tool.name}
                  className="flex items-center justify-between grid grid-cols-12"
                >
                  <div className="flex flex-col col-span-8">
                    <label className="block text-xs font-medium text-textStandard">
                      {tool.name}
                    </label>
                    <p className="text-xs text-textSubtle mb-2">
                      {getToolDescription(tool.name, tool.description)}
                    </p>
                  </div>
                  <DropdownMenu>
                    <DropdownMenuTrigger className="col-span-4">
                      <Button className="w-full" variant="secondary" size="lg">
                        {permissionOptions.find(
                          (option) =>
                            option.value === (updatedPermissions[tool.name] || tool.permission)
                        )?.label || t('permission.askBefore')}
                        <ChevronDownIcon className="h-4 w-4" />
                      </Button>
                    </DropdownMenuTrigger>
                    <DropdownMenuContent>
                      {permissionOptions.map((option) => (
                        <DropdownMenuItem
                          key={option.value}
                          onSelect={() =>
                            handleSettingChange(tool.name, option.value as PermissionLevel)
                          }
                        >
                          {option.label}
                        </DropdownMenuItem>
                      ))}
                    </DropdownMenuContent>
                  </DropdownMenu>
                </div>
              ))}
            </div>
          )}
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={handleClose}>
            {tCommon('cancel')}
          </Button>
          <Button disabled={!hasChanges} onClick={handleSave}>
            {t('permission.saveChanges')}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
