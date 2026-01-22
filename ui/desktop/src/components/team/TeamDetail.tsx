import React, { useState, useEffect, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import {
  ArrowLeft,
  Users,
  Sparkles,
  Book,
  Puzzle,
  Download,
  Check,
  AlertTriangle,
  Plus,
  MoreVertical,
  Trash2,
  CheckCircle,
  XCircle,
  Eye,
  Pencil,
  UserPlus,
  LogOut,
  Settings,
  Cloud,
  ChevronDown,
  Lock,
  Link,
} from 'lucide-react';
import { toastService } from '../../toasts';
import {
  TeamSummary,
  TeamDetailTab,
  TeamMember,
  SharedSkill,
  SharedRecipe,
  SharedExtension,
  ProtectionLevel,
  allowsLocalInstall,
  isBuiltinExtension,
} from './types';
import {
  listMembers,
  listSkills,
  listRecipes,
  listExtensions,
  installSkill,
  installRecipe,
  installExtension,
  deleteSkill,
  deleteRecipe,
  deleteExtension,
  reviewExtension,
  getSkill,
  getRecipe,
  getExtension,
  addMember,
  updateMember,
  removeMemberWithCleanup,
  leaveTeam,
  updateTeam,
  uninstallSkill,
  uninstallRecipe,
  uninstallExtension,
  getCleanupCount,
} from './api';
import { Button } from '../ui/button';
import { Input } from '../ui/input';
import { ShareResourceDialog } from './ShareResourceDialog';
import ResourceDetailDialog from './ResourceDetailDialog';
import ResourceEditDialog from './ResourceEditDialog';
import SyncStatusIndicator from './SyncStatusIndicator';
import { useConfig } from '../ConfigContext';
import type { ExtensionConfig as AgimeExtensionConfig } from '../../api/types.gen';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '../ui/dropdown-menu';
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
  DialogDescription,
} from '../ui/dialog';
import { Textarea } from '../ui/textarea';
import { InviteMemberDialog, InviteListDialog } from './invites';

interface TeamDetailProps {
  teamSummary: TeamSummary;
  isLoading: boolean;
  error: string | null;
  onBack: () => void;
  onRetry: () => void;
  onTeamDeleted: (teamId: string) => void;
  onTeamUpdated?: (team: TeamSummary) => void;
  currentUserId?: string;
}

// Type for delete confirmation state
interface DeleteConfirmState {
  type: 'skill' | 'recipe' | 'extension';
  id: string;
  name: string;
}

// Type for review confirmation state
interface ReviewConfirmState {
  extensionId: string;
  name: string;
  approved: boolean;
}

// Type for detail view state
interface DetailViewState {
  type: 'skill' | 'recipe' | 'extension';
  resource: SharedSkill | SharedRecipe | SharedExtension | null;
  isLoading: boolean;
  error: string | null;
}

// Type for edit state
interface EditState {
  type: 'skill' | 'recipe' | 'extension';
  resource: SharedSkill | SharedRecipe | SharedExtension | null;
}

// Type for add member state
interface AddMemberState {
  userId: string;
  displayName: string;
  role: 'admin' | 'member';
}

// Type for edit member role state
interface EditMemberRoleState {
  memberId: string;
  memberName: string;
  currentRole: string;
  newRole: string;
}

// Type for team edit state
interface TeamEditState {
  name: string;
  description: string;
}

const TeamDetail: React.FC<TeamDetailProps> = ({
  teamSummary,
  isLoading,
  error,
  onBack,
  onRetry,
  onTeamUpdated,
  currentUserId,
}) => {
  const { t } = useTranslation('team');
  const { addExtension: registerAgimeExtension } = useConfig();
  const [activeTab, setActiveTab] = useState<TeamDetailTab>('members');
  const [members, setMembers] = useState<TeamMember[]>([]);
  const [skills, setSkills] = useState<SharedSkill[]>([]);
  const [recipes, setRecipes] = useState<SharedRecipe[]>([]);
  const [extensions, setExtensions] = useState<SharedExtension[]>([]);
  const [isLoadingTab, setIsLoadingTab] = useState(false);
  const [installingId, setInstallingId] = useState<string | null>(null);
  const [shareDialogOpen, setShareDialogOpen] = useState(false);
  const [shareResourceType, setShareResourceType] = useState<'skill' | 'recipe' | 'extension'>('skill');

  // Management states
  const [deleteConfirm, setDeleteConfirm] = useState<DeleteConfirmState | null>(null);
  const [reviewConfirm, setReviewConfirm] = useState<ReviewConfirmState | null>(null);
  const [reviewNotes, setReviewNotes] = useState('');
  const [isDeleting, setIsDeleting] = useState(false);
  const [isReviewing, setIsReviewing] = useState(false);

  // Detail and edit states
  const [detailView, setDetailView] = useState<DetailViewState | null>(null);
  const [editState, setEditState] = useState<EditState | null>(null);

  // Member management states
  const [editMemberRole, setEditMemberRole] = useState<EditMemberRoleState | null>(null);
  const [isUpdatingRole, setIsUpdatingRole] = useState(false);
  const [removeMemberConfirm, setRemoveMemberConfirm] = useState<{ id: string; name: string; userId: string } | null>(null);
  const [isRemovingMember, setIsRemovingMember] = useState(false);
  const [cleanupCount, setCleanupCount] = useState<number | null>(null);
  const [isLoadingCleanupCount, setIsLoadingCleanupCount] = useState(false);
  const [showLeaveConfirm, setShowLeaveConfirm] = useState(false);
  const [isLeaving, setIsLeaving] = useState(false);

  // Team edit states
  const [showTeamEdit, setShowTeamEdit] = useState(false);
  const [teamEditData, setTeamEditData] = useState<TeamEditState>({ name: '', description: '' });
  const [isUpdatingTeam, setIsUpdatingTeam] = useState(false);

  // Invite dialog state
  const [showInviteDialog, setShowInviteDialog] = useState(false);
  const [showInviteListDialog, setShowInviteListDialog] = useState(false);

  // Uninstall states
  const [uninstallConfirm, setUninstallConfirm] = useState<DeleteConfirmState | null>(null);
  const [isUninstalling, setIsUninstalling] = useState(false);

  // Add member direct states
  const [showAddMember, setShowAddMember] = useState(false);
  const [addMemberData, setAddMemberData] = useState<AddMemberState>({ userId: '', displayName: '', role: 'member' });
  const [isAddingMember, setIsAddingMember] = useState(false);

  const { team } = teamSummary;

  // Find the current user's member info to check permissions
  const currentMember = members.find((m) => m.userId === currentUserId);
  const isAdminOrOwner = currentMember?.role === 'owner' || currentMember?.role === 'admin';

  const loadTabData = useCallback(async () => {
    setIsLoadingTab(true);
    try {
      switch (activeTab) {
        case 'members':
          const membersResponse = await listMembers(team.id);
          setMembers(membersResponse.members);
          break;
        case 'skills':
          const skillsResponse = await listSkills({ teamId: team.id });
          setSkills(skillsResponse.skills);
          break;
        case 'recipes':
          const recipesResponse = await listRecipes({ teamId: team.id });
          setRecipes(recipesResponse.recipes);
          break;
        case 'extensions':
          const extensionsResponse = await listExtensions({ teamId: team.id });
          setExtensions(extensionsResponse.extensions);
          break;
      }
    } catch (err) {
      console.error(`Failed to load ${activeTab}:`, err);
    } finally {
      setIsLoadingTab(false);
    }
  }, [activeTab, team.id]);

  useEffect(() => {
    loadTabData();
  }, [loadTabData]);

  // Fetch cleanup count when remove member dialog opens
  useEffect(() => {
    if (removeMemberConfirm) {
      setIsLoadingCleanupCount(true);
      setCleanupCount(null);
      getCleanupCount(team.id, removeMemberConfirm.userId)
        .then((response) => {
          setCleanupCount(response.count);
        })
        .catch((err) => {
          console.error('Failed to get cleanup count:', err);
          setCleanupCount(0);
        })
        .finally(() => {
          setIsLoadingCleanupCount(false);
        });
    }
  }, [removeMemberConfirm, team.id]);

  const handleInstall = async (type: 'skill' | 'recipe' | 'extension', id: string) => {
    setInstallingId(id);
    try {
      switch (type) {
        case 'skill':
          await installSkill(id);
          break;
        case 'recipe':
          await installRecipe(id);
          break;
        case 'extension': {
          await installExtension(id);
          // After successful installation, register the extension to AGIME config
          try {
            const ext = await getExtension(id);
            // Convert team extension config to AGIME extension config
            const agimeConfig = convertToAgimeExtensionConfig(ext);
            if (agimeConfig) {
              await registerAgimeExtension(ext.name, agimeConfig, true);
            }
          } catch (regErr) {
            console.error('Failed to register extension to AGIME config:', regErr);
            // Installation succeeded, but registration failed - still show success
            // The extension files are saved, user can manually enable it later
          }
          break;
        }
      }
      toastService.success({
        title: t('installSuccess.title'),
        msg: t('installSuccess.message', { type: t(`resources.${type}`) }),
      });
    } catch (err) {
      console.error(`Failed to install ${type}:`, err);
      toastService.error({
        title: t('installFailed.title'),
        msg: err instanceof Error ? err.message : t('installFailed.message'),
      });
    } finally {
      setInstallingId(null);
    }
  };

  // Convert team extension config to AGIME extension config format
  const convertToAgimeExtensionConfig = (ext: SharedExtension): AgimeExtensionConfig | null => {
    const description = ext.description || `Team extension: ${ext.name}`;

    switch (ext.extensionType) {
      case 'stdio':
        if (!ext.config.command) {
          console.warn('Stdio extension missing command:', ext.name);
          return null;
        }
        return {
          type: 'stdio',
          name: ext.name,
          description,
          cmd: ext.config.command,
          args: ext.config.args || [],
          envs: ext.config.env || {},
        };
      case 'sse':
        if (!ext.config.url) {
          console.warn('SSE extension missing URL:', ext.name);
          return null;
        }
        return {
          type: 'sse',
          name: ext.name,
          description,
          uri: ext.config.url,
          envs: ext.config.env || {},
        };
      case 'builtin':
        return {
          type: 'builtin',
          name: ext.name,
          description,
        };
      default:
        console.warn('Unknown extension type:', ext.extensionType);
        return null;
    }
  };

  // Handle resource deletion
  const handleDelete = async () => {
    if (!deleteConfirm) return;

    setIsDeleting(true);
    try {
      switch (deleteConfirm.type) {
        case 'skill':
          await deleteSkill(deleteConfirm.id);
          setSkills((prev) => prev.filter((s) => s.id !== deleteConfirm.id));
          break;
        case 'recipe':
          await deleteRecipe(deleteConfirm.id);
          setRecipes((prev) => prev.filter((r) => r.id !== deleteConfirm.id));
          break;
        case 'extension':
          await deleteExtension(deleteConfirm.id);
          setExtensions((prev) => prev.filter((e) => e.id !== deleteConfirm.id));
          break;
      }
      setDeleteConfirm(null);
    } catch (err) {
      const errorMessage = err instanceof Error ? err.message : String(err);
      // If resource not found (404), remove it from the list anyway (data out of sync)
      if (errorMessage.includes('not found') || errorMessage.includes('404')) {
        console.warn(`Resource ${deleteConfirm.type} ${deleteConfirm.id} not found on server, removing from local list`);
        switch (deleteConfirm.type) {
          case 'skill':
            setSkills((prev) => prev.filter((s) => s.id !== deleteConfirm.id));
            break;
          case 'recipe':
            setRecipes((prev) => prev.filter((r) => r.id !== deleteConfirm.id));
            break;
          case 'extension':
            setExtensions((prev) => prev.filter((e) => e.id !== deleteConfirm.id));
            break;
        }
        setDeleteConfirm(null);
      } else {
        console.error(`Failed to delete ${deleteConfirm.type}:`, err);
      }
    } finally {
      setIsDeleting(false);
    }
  };

  // Handle extension review
  const handleReview = async () => {
    if (!reviewConfirm) return;

    setIsReviewing(true);
    try {
      const updated = await reviewExtension(reviewConfirm.extensionId, {
        approved: reviewConfirm.approved,
        notes: reviewNotes || undefined,
      });
      // Update the extension in state
      setExtensions((prev) =>
        prev.map((e) =>
          e.id === updated.id
            ? {
              ...e,
              securityReviewed: updated.securityReviewed,
              securityNotes: updated.securityNotes,
              reviewedBy: updated.reviewedBy,
              reviewedAt: updated.reviewedAt,
            }
            : e
        )
      );
      setReviewConfirm(null);
      setReviewNotes('');
    } catch (err) {
      console.error('Failed to review extension:', err);
    } finally {
      setIsReviewing(false);
    }
  };

  // Check if user can delete a resource
  const canDelete = (authorId: string): boolean => {
    if (isAdminOrOwner) return true;
    return currentUserId === authorId;
  };

  // Check if user can edit a resource (same as delete permission)
  const canEdit = canDelete;

  // Handle viewing resource details
  const handleViewDetail = async (type: 'skill' | 'recipe' | 'extension', id: string) => {
    setDetailView({ type, resource: null, isLoading: true, error: null });
    try {
      let resource: SharedSkill | SharedRecipe | SharedExtension;
      switch (type) {
        case 'skill':
          resource = await getSkill(id);
          break;
        case 'recipe':
          resource = await getRecipe(id);
          break;
        case 'extension':
          resource = await getExtension(id);
          break;
      }
      setDetailView({ type, resource, isLoading: false, error: null });
    } catch (err) {
      console.error(`Failed to load ${type} details:`, err);
      setDetailView({ type, resource: null, isLoading: false, error: t('manage.loadDetailError') });
    }
  };

  // Handle opening edit dialog
  const handleOpenEdit = async (type: 'skill' | 'recipe' | 'extension', id: string) => {
    try {
      let resource: SharedSkill | SharedRecipe | SharedExtension;
      switch (type) {
        case 'skill':
          resource = await getSkill(id);
          break;
        case 'recipe':
          resource = await getRecipe(id);
          break;
        case 'extension':
          resource = await getExtension(id);
          break;
      }
      setEditState({ type, resource });
    } catch (err) {
      console.error(`Failed to load ${type} for editing:`, err);
    }
  };

  // Handle successful edit
  const handleEditSuccess = (updated: SharedSkill | SharedRecipe | SharedExtension) => {
    // Update the local state with the updated resource
    if (editState?.type === 'skill') {
      setSkills((prev) => prev.map((s) => (s.id === updated.id ? (updated as SharedSkill) : s)));
    } else if (editState?.type === 'recipe') {
      setRecipes((prev) => prev.map((r) => (r.id === updated.id ? (updated as SharedRecipe) : r)));
    } else if (editState?.type === 'extension') {
      setExtensions((prev) => prev.map((e) => (e.id === updated.id ? (updated as SharedExtension) : e)));
    }
  };

  // Handle adding a new member
  const handleAddMember = async () => {
    if (!addMemberData.userId.trim() || !addMemberData.displayName.trim()) return;

    setIsAddingMember(true);
    try {
      const newMember = await addMember(team.id, {
        userId: addMemberData.userId.trim(),
        displayName: addMemberData.displayName.trim(),
        role: addMemberData.role,
      });
      setMembers((prev) => [...prev, newMember]);
      setShowAddMember(false);
      setAddMemberData({ userId: '', displayName: '', role: 'member' });
    } catch (err) {
      console.error('Failed to add member:', err);
    } finally {
      setIsAddingMember(false);
    }
  };

  // Handle updating member role
  const handleUpdateMemberRole = async () => {
    if (!editMemberRole) return;

    setIsUpdatingRole(true);
    try {
      const updated = await updateMember(editMemberRole.memberId, { role: editMemberRole.newRole });
      setMembers((prev) => prev.map((m) => (m.id === updated.id ? updated : m)));
      setEditMemberRole(null);
    } catch (err) {
      console.error('Failed to update member role:', err);
    } finally {
      setIsUpdatingRole(false);
    }
  };

  // Handle removing a member
  const handleRemoveMember = async () => {
    if (!removeMemberConfirm) return;

    setIsRemovingMember(true);
    try {
      const result = await removeMemberWithCleanup(removeMemberConfirm.id);
      // Log cleanup result for debugging
      console.log('Member removed with cleanup:', {
        memberId: result.memberId,
        cleanedCount: result.cleanedCount,
        failures: result.failures,
      });
      setMembers((prev) => prev.filter((m) => m.id !== removeMemberConfirm.id));
      setRemoveMemberConfirm(null);
      // Note: Client-side local files will be cleaned up on next skill load
      // when skills_extension verifies authorization
    } catch (err) {
      console.error('Failed to remove member:', err);
    } finally {
      setIsRemovingMember(false);
    }
  };

  // Handle leaving team
  const handleLeaveTeam = async () => {
    setIsLeaving(true);
    try {
      await leaveTeam(team.id);
      setShowLeaveConfirm(false);
      onBack();
    } catch (err) {
      console.error('Failed to leave team:', err);
    } finally {
      setIsLeaving(false);
    }
  };

  // Handle opening team edit dialog
  const handleOpenTeamEdit = () => {
    setTeamEditData({ name: team.name, description: team.description || '' });
    setShowTeamEdit(true);
  };

  // Handle updating team
  const handleUpdateTeam = async () => {
    if (!teamEditData.name.trim()) return;

    setIsUpdatingTeam(true);
    try {
      const updated = await updateTeam(team.id, {
        name: teamEditData.name.trim(),
        description: teamEditData.description.trim() || undefined,
      });
      // Update teamSummary through callback
      if (onTeamUpdated) {
        onTeamUpdated({ ...teamSummary, team: updated });
      }
      setShowTeamEdit(false);
    } catch (err) {
      console.error('Failed to update team:', err);
    } finally {
      setIsUpdatingTeam(false);
    }
  };

  // Handle uninstalling a resource
  const handleUninstall = async () => {
    if (!uninstallConfirm) return;

    setIsUninstalling(true);
    try {
      switch (uninstallConfirm.type) {
        case 'skill':
          await uninstallSkill(uninstallConfirm.id);
          break;
        case 'recipe':
          await uninstallRecipe(uninstallConfirm.id);
          break;
        case 'extension':
          await uninstallExtension(uninstallConfirm.id);
          break;
      }
      setUninstallConfirm(null);
    } catch (err) {
      console.error(`Failed to uninstall ${uninstallConfirm.type}:`, err);
    } finally {
      setIsUninstalling(false);
    }
  };

  const tabs: { key: TeamDetailTab; label: string; icon: React.ElementType; count: number }[] = [
    { key: 'members', label: t('members'), icon: Users, count: teamSummary.membersCount },
    { key: 'skills', label: t('skills'), icon: Sparkles, count: teamSummary.skillsCount },
    { key: 'recipes', label: t('recipes'), icon: Book, count: teamSummary.recipesCount },
    { key: 'extensions', label: t('extensions'), icon: Puzzle, count: teamSummary.extensionsCount },
  ];

  if (isLoading) {
    return (
      <div className="flex items-center justify-center h-64">
        <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-teal-500"></div>
      </div>
    );
  }

  if (error) {
    return (
      <div className="flex flex-col items-center justify-center h-64">
        <p className="text-red-500 mb-4">{error}</p>
        <div className="flex gap-3">
          <Button variant="outline" onClick={onBack}>
            {t('back')}
          </Button>
          <Button onClick={onRetry}>{t('retry')}</Button>
        </div>
      </div>
    );
  }

  return (
    <div className="flex flex-col h-full bg-background-default">
      {/* Top navigation bar */}
      <div className="flex items-center justify-between px-6 pt-14 pb-4">
        <button
          onClick={onBack}
          className="flex items-center gap-2 text-text-muted hover:text-text-default transition-colors"
        >
          <ArrowLeft size={18} />
          <span className="text-sm">{t('backToTeams')}</span>
        </button>

        {/* Right: Actions */}
        <div className="flex items-center gap-1">
          {/* Share dropdown */}
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button variant="ghost" size="sm" className="h-8 gap-1.5 text-text-muted hover:text-text-default">
                <Plus size={14} />
                <span>{t('toolbar.share', '分享')}</span>
                <ChevronDown size={12} />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              <DropdownMenuItem onClick={() => { setShareResourceType('skill'); setShareDialogOpen(true); }}>
                <Sparkles size={14} />
                {t('quickActions.shareSkill')}
              </DropdownMenuItem>
              <DropdownMenuItem onClick={() => { setShareResourceType('recipe'); setShareDialogOpen(true); }}>
                <Book size={14} />
                {t('quickActions.shareRecipe')}
              </DropdownMenuItem>
              <DropdownMenuItem onClick={() => { setShareResourceType('extension'); setShareDialogOpen(true); }}>
                <Puzzle size={14} />
                {t('quickActions.shareExtension')}
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>

          {/* Sync status */}
          <SyncStatusIndicator teamId={team.id} onSyncComplete={loadTabData} />

          {/* Settings */}
          {isAdminOrOwner && (
            <Button variant="ghost" size="sm" onClick={handleOpenTeamEdit} className="h-7 w-7 p-0 text-text-muted hover:text-text-default">
              <Settings size={16} />
            </Button>
          )}

          {/* Leave team */}
          {currentMember && currentMember.role !== 'owner' && (
            <Button
              variant="ghost"
              size="sm"
              onClick={() => setShowLeaveConfirm(true)}
              className="h-7 w-7 p-0 text-red-500 hover:text-red-600 hover:bg-red-500/10"
            >
              <LogOut size={16} />
            </Button>
          )}
        </div>
      </div>

      {/* Team Header */}
      <div className="px-6 pb-6">
        <h1 className="text-2xl sm:text-3xl font-semibold text-text-default mb-1">{team.name}</h1>
        {team.description && (
          <p className="text-sm text-text-muted">{team.description}</p>
        )}
      </div>

      {/* Tabs */}
      <div className="px-6 border-b border-border-subtle">
        <div className="flex gap-1">
          {tabs.map(({ key, label, icon: Icon, count }) => {
            const isActive = activeTab === key;
            return (
              <button
                key={key}
                onClick={() => setActiveTab(key)}
                className={`
                  flex items-center gap-2 px-4 py-3 text-sm transition-colors relative
                  ${isActive
                    ? 'text-text-default'
                    : 'text-text-muted hover:text-text-default'
                  }
                `}
              >
                <Icon size={16} />
                <span>{label}</span>
                {count > 0 && (
                  <span className="text-xs text-text-muted">{count}</span>
                )}
                {isActive && (
                  <div className="absolute bottom-0 left-0 right-0 h-0.5 bg-text-default" />
                )}
              </button>
            );
          })}
        </div>
      </div>

      {/* Tab content */}
      <div className="flex-1 overflow-y-auto">
        <div className="p-6">
          {isLoadingTab ? (
            <div className="flex items-center justify-center h-32">
              <div className="animate-spin rounded-full h-6 w-6 border-b-2 border-text-muted"></div>
            </div>
          ) : (
            <>
              {activeTab === 'members' && (
                <>
                  {/* Member Management Toolbar */}
                  {isAdminOrOwner && (
                    <div className="flex justify-end gap-2 mb-4">
                      <Button
                        onClick={() => setShowInviteDialog(true)}
                        variant="default"
                        size="sm"
                        className="gap-1.5"
                      >
                        <UserPlus size={14} />
                        {t('membersTab.invite', '邀请成员')}
                      </Button>
                      <Button
                        onClick={() => setShowInviteListDialog(true)}
                        variant="outline"
                        size="sm"
                        className="gap-1.5"
                      >
                        <Link size={14} />
                        {t('membersTab.viewInvites', '查看邀请')}
                      </Button>
                    </div>
                  )}

                  <MembersListBento
                    members={members}
                    currentUserId={currentUserId}
                    isAdminOrOwner={isAdminOrOwner}
                    onEditRole={(member) => setEditMemberRole({
                      memberId: member.id,
                      memberName: member.displayName,
                      currentRole: member.role,
                      newRole: member.role,
                    })}
                    onRemove={(member) => setRemoveMemberConfirm({ id: member.id, name: member.displayName, userId: member.userId })}
                    onAddMember={() => setShowAddMember(true)}
                  />
                </>
              )}
              {activeTab === 'skills' && (
                <SkillsListBento
                  skills={skills}
                  onInstall={(id) => handleInstall('skill', id)}
                  installingId={installingId}
                  canDelete={canDelete}
                  canEdit={canEdit}
                  onDelete={(skill) => setDeleteConfirm({ type: 'skill', id: skill.id, name: skill.name })}
                  onViewDetail={(skill) => handleViewDetail('skill', skill.id)}
                  onEdit={(skill) => handleOpenEdit('skill', skill.id)}
                  onShare={() => { setShareResourceType('skill'); setShareDialogOpen(true); }}
                />
              )}
              {activeTab === 'recipes' && (
                <RecipesListBento
                  recipes={recipes}
                  onInstall={(id) => handleInstall('recipe', id)}
                  installingId={installingId}
                  canDelete={canDelete}
                  canEdit={canEdit}
                  onDelete={(recipe) => setDeleteConfirm({ type: 'recipe', id: recipe.id, name: recipe.name })}
                  onViewDetail={(recipe) => handleViewDetail('recipe', recipe.id)}
                  onEdit={(recipe) => handleOpenEdit('recipe', recipe.id)}
                  onShare={() => { setShareResourceType('recipe'); setShareDialogOpen(true); }}
                />
              )}
              {activeTab === 'extensions' && (
                <ExtensionsListBento
                  extensions={extensions}
                  onInstall={(id) => handleInstall('extension', id)}
                  installingId={installingId}
                  canDelete={canDelete}
                  canEdit={canEdit}
                  onDelete={(ext) => setDeleteConfirm({ type: 'extension', id: ext.id, name: ext.name })}
                  onViewDetail={(ext) => handleViewDetail('extension', ext.id)}
                  onEdit={(ext) => handleOpenEdit('extension', ext.id)}
                  canReview={isAdminOrOwner}
                  onReview={(ext, approved) => setReviewConfirm({ extensionId: ext.id, name: ext.name, approved })}
                  onShare={() => { setShareResourceType('extension'); setShareDialogOpen(true); }}
                />
              )}
            </>
          )}
        </div>
      </div>
      {/* Share dialog */}
      <ShareResourceDialog
        open={shareDialogOpen}
        onOpenChange={setShareDialogOpen}
        teamId={team.id}
        resourceType={shareResourceType}
        onSuccess={loadTabData}
      />

      {/* Delete confirmation dialog */}
      <Dialog open={!!deleteConfirm} onOpenChange={() => setDeleteConfirm(null)}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>{t('manage.delete')}</DialogTitle>
            <DialogDescription>
              {deleteConfirm?.type === 'skill' && t('manage.deleteSkillConfirm')}
              {deleteConfirm?.type === 'recipe' && t('manage.deleteRecipeConfirm')}
              {deleteConfirm?.type === 'extension' && t('manage.deleteExtensionConfirm')}
            </DialogDescription>
          </DialogHeader>
          <div className="py-4">
            <p className="text-sm text-text-default font-medium">{deleteConfirm?.name}</p>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleteConfirm(null)} disabled={isDeleting}>
              {t('cancel')}
            </Button>
            <Button variant="destructive" onClick={handleDelete} disabled={isDeleting}>
              {isDeleting ? (
                <div className="animate-spin h-4 w-4 border-2 border-white border-t-transparent rounded-full mr-2" />
              ) : (
                <Trash2 size={14} className="mr-2" />
              )}
              {t('manage.delete')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Review confirmation dialog */}
      <Dialog open={!!reviewConfirm} onOpenChange={() => { setReviewConfirm(null); setReviewNotes(''); }}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>
              {reviewConfirm?.approved ? t('manage.approve') : t('manage.reject')}
            </DialogTitle>
            <DialogDescription>
              {reviewConfirm?.approved ? t('manage.confirmApprove') : t('manage.confirmReject')}
            </DialogDescription>
          </DialogHeader>
          <div className="py-4 space-y-4">
            <p className="text-sm text-text-default font-medium">{reviewConfirm?.name}</p>
            <div>
              <label className="text-sm text-text-muted mb-1.5 block">
                {t('manage.reviewNotes')}
              </label>
              <Textarea
                value={reviewNotes}
                onChange={(e) => setReviewNotes(e.target.value)}
                placeholder={t('manage.reviewNotesPlaceholder')}
                rows={3}
              />
            </div>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => { setReviewConfirm(null); setReviewNotes(''); }} disabled={isReviewing}>
              {t('cancel')}
            </Button>
            <Button
              variant={reviewConfirm?.approved ? 'default' : 'destructive'}
              onClick={handleReview}
              disabled={isReviewing}
            >
              {isReviewing ? (
                <div className="animate-spin h-4 w-4 border-2 border-white border-t-transparent rounded-full mr-2" />
              ) : reviewConfirm?.approved ? (
                <CheckCircle size={14} className="mr-2" />
              ) : (
                <XCircle size={14} className="mr-2" />
              )}
              {reviewConfirm?.approved ? t('manage.approve') : t('manage.reject')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Invite Member Dialog */}
      <InviteMemberDialog
        open={showInviteDialog}
        onClose={() => setShowInviteDialog(false)}
        teamId={team.id}
        teamName={team.name}
      />

      {/* Invite List Dialog */}
      <InviteListDialog
        open={showInviteListDialog}
        onClose={() => setShowInviteListDialog(false)}
        teamId={team.id}
        teamName={team.name}
      />

      {/* Resource detail dialog */}
      <ResourceDetailDialog
        open={!!detailView}
        onOpenChange={(open) => !open && setDetailView(null)}
        resourceType={detailView?.type || 'skill'}
        resource={detailView?.resource || null}
        isLoading={detailView?.isLoading}
        error={detailView?.error}
      />

      {/* Resource edit dialog */}
      <ResourceEditDialog
        open={!!editState}
        onOpenChange={(open) => !open && setEditState(null)}
        resourceType={editState?.type || 'skill'}
        resource={editState?.resource || null}
        onSuccess={handleEditSuccess}
      />

      {/* Add member dialog */}
      <Dialog open={showAddMember} onOpenChange={setShowAddMember}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>{t('memberManage.addMemberTitle')}</DialogTitle>
          </DialogHeader>
          <div className="space-y-4 py-4">
            <div>
              <label className="text-sm font-medium text-text-default mb-1.5 block">
                {t('memberManage.userId')} *
              </label>
              <Input
                value={addMemberData.userId}
                onChange={(e) => setAddMemberData({ ...addMemberData, userId: e.target.value })}
                placeholder={t('memberManage.userIdPlaceholder')}
              />
            </div>
            <div>
              <label className="text-sm font-medium text-text-default mb-1.5 block">
                {t('memberManage.displayName')} *
              </label>
              <Input
                value={addMemberData.displayName}
                onChange={(e) => setAddMemberData({ ...addMemberData, displayName: e.target.value })}
                placeholder={t('memberManage.displayNamePlaceholder')}
              />
            </div>
            <div>
              <label className="text-sm font-medium text-text-default mb-1.5 block">
                {t('memberManage.role')}
              </label>
              <select
                value={addMemberData.role}
                onChange={(e) => setAddMemberData({ ...addMemberData, role: e.target.value as 'admin' | 'member' })}
                className="w-full px-3 py-2 text-sm border border-border-default rounded-md bg-background-default text-text-default"
              >
                <option value="member">{t('roles.member')}</option>
                <option value="admin">{t('roles.admin')}</option>
              </select>
            </div>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setShowAddMember(false)} disabled={isAddingMember}>
              {t('cancel')}
            </Button>
            <Button onClick={handleAddMember} disabled={isAddingMember || !addMemberData.userId.trim() || !addMemberData.displayName.trim()}>
              {isAddingMember ? (
                <div className="animate-spin h-4 w-4 border-2 border-white border-t-transparent rounded-full mr-2" />
              ) : (
                <UserPlus size={14} className="mr-2" />
              )}
              {t('memberManage.addMember')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Edit member role dialog */}
      <Dialog open={!!editMemberRole} onOpenChange={() => setEditMemberRole(null)}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>{t('memberManage.editRoleTitle')}</DialogTitle>
          </DialogHeader>
          <div className="space-y-4 py-4">
            <p className="text-sm text-text-default">{editMemberRole?.memberName}</p>
            <div>
              <label className="text-sm font-medium text-text-default mb-1.5 block">
                {t('memberManage.role')}
              </label>
              <select
                value={editMemberRole?.newRole || ''}
                onChange={(e) => editMemberRole && setEditMemberRole({ ...editMemberRole, newRole: e.target.value })}
                className="w-full px-3 py-2 text-sm border border-border-default rounded-md bg-background-default text-text-default"
              >
                <option value="member">{t('roles.member')}</option>
                <option value="admin">{t('roles.admin')}</option>
              </select>
            </div>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setEditMemberRole(null)} disabled={isUpdatingRole}>
              {t('cancel')}
            </Button>
            <Button onClick={handleUpdateMemberRole} disabled={isUpdatingRole || editMemberRole?.newRole === editMemberRole?.currentRole}>
              {isUpdatingRole && <div className="animate-spin h-4 w-4 border-2 border-white border-t-transparent rounded-full mr-2" />}
              {t('manage.save')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Remove member confirmation dialog */}
      <Dialog open={!!removeMemberConfirm} onOpenChange={() => setRemoveMemberConfirm(null)}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>{t('memberManage.remove')}</DialogTitle>
            <DialogDescription>{t('memberManage.removeConfirm')}</DialogDescription>
          </DialogHeader>
          <div className="py-4 space-y-3">
            <p className="text-sm text-text-default font-medium">{removeMemberConfirm?.name}</p>
            {/* Cleanup info */}
            <div className="p-3 bg-background-muted rounded-lg">
              <div className="flex items-center gap-2 text-sm">
                {isLoadingCleanupCount ? (
                  <>
                    <div className="animate-spin h-4 w-4 border-2 border-teal-500 border-t-transparent rounded-full" />
                    <span className="text-text-muted">{t('memberManage.loadingCleanup', '正在检查安装的资源...')}</span>
                  </>
                ) : cleanupCount !== null && cleanupCount > 0 ? (
                  <>
                    <AlertTriangle size={16} className="text-yellow-500" />
                    <span className="text-yellow-600 dark:text-yellow-400">
                      {t('memberManage.cleanupWarning', '删除后将自动卸载该用户安装的 {{count}} 个团队资源', { count: cleanupCount })}
                    </span>
                  </>
                ) : (
                  <>
                    <Check size={16} className="text-green-500" />
                    <span className="text-text-muted">{t('memberManage.noCleanup', '该用户没有安装团队资源')}</span>
                  </>
                )}
              </div>
            </div>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setRemoveMemberConfirm(null)} disabled={isRemovingMember}>
              {t('cancel')}
            </Button>
            <Button variant="destructive" onClick={handleRemoveMember} disabled={isRemovingMember || isLoadingCleanupCount}>
              {isRemovingMember && <div className="animate-spin h-4 w-4 border-2 border-white border-t-transparent rounded-full mr-2" />}
              {t('memberManage.remove')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Leave team confirmation dialog */}
      <Dialog open={showLeaveConfirm} onOpenChange={setShowLeaveConfirm}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>{t('memberManage.leaveTeam')}</DialogTitle>
            <DialogDescription>{t('memberManage.leaveTeamConfirm')}</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setShowLeaveConfirm(false)} disabled={isLeaving}>
              {t('cancel')}
            </Button>
            <Button variant="destructive" onClick={handleLeaveTeam} disabled={isLeaving}>
              {isLeaving && <div className="animate-spin h-4 w-4 border-2 border-white border-t-transparent rounded-full mr-2" />}
              <LogOut size={14} className="mr-2" />
              {t('memberManage.leaveTeam')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Team edit dialog */}
      <Dialog open={showTeamEdit} onOpenChange={setShowTeamEdit}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>{t('team.editTitle')}</DialogTitle>
          </DialogHeader>
          <div className="space-y-4 py-4">
            <div>
              <label className="text-sm font-medium text-text-default mb-1.5 block">
                {t('teamName')} *
              </label>
              <Input
                value={teamEditData.name}
                onChange={(e) => setTeamEditData({ ...teamEditData, name: e.target.value })}
                placeholder={t('teamNamePlaceholder')}
              />
            </div>
            <div>
              <label className="text-sm font-medium text-text-default mb-1.5 block">
                {t('description')}
              </label>
              <Textarea
                value={teamEditData.description}
                onChange={(e) => setTeamEditData({ ...teamEditData, description: e.target.value })}
                placeholder={t('descriptionPlaceholder')}
                rows={3}
              />
            </div>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setShowTeamEdit(false)} disabled={isUpdatingTeam}>
              {t('cancel')}
            </Button>
            <Button onClick={handleUpdateTeam} disabled={isUpdatingTeam || !teamEditData.name.trim()}>
              {isUpdatingTeam && <div className="animate-spin h-4 w-4 border-2 border-white border-t-transparent rounded-full mr-2" />}
              {t('manage.save')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Uninstall confirmation dialog */}
      <Dialog open={!!uninstallConfirm} onOpenChange={() => setUninstallConfirm(null)}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>{t('uninstall.title')}</DialogTitle>
            <DialogDescription>{t('uninstall.confirm')}</DialogDescription>
          </DialogHeader>
          <div className="py-4">
            <p className="text-sm text-text-default font-medium">{uninstallConfirm?.name}</p>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setUninstallConfirm(null)} disabled={isUninstalling}>
              {t('cancel')}
            </Button>
            <Button variant="destructive" onClick={handleUninstall} disabled={isUninstalling}>
              {isUninstalling && <div className="animate-spin h-4 w-4 border-2 border-white border-t-transparent rounded-full mr-2" />}
              {t('uninstall.title')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
};

// Protection Level Badge component
const ProtectionLevelBadge: React.FC<{ level: ProtectionLevel }> = ({ level }) => {
  const { t } = useTranslation('team');

  const config: Record<ProtectionLevel, { label: string; className: string }> = {
    public: {
      label: t('protectionLevel.public', '公开'),
      className: 'bg-background-muted text-text-muted',
    },
    team_installable: {
      label: t('protectionLevel.teamInstallable', '可安装'),
      className: 'bg-background-muted text-text-muted',
    },
    team_online_only: {
      label: t('protectionLevel.teamOnlineOnly', '仅在线'),
      className: 'bg-amber-500/10 text-amber-600 dark:text-amber-400',
    },
    controlled: {
      label: t('protectionLevel.controlled', '受控'),
      className: 'bg-red-500/10 text-red-600 dark:text-red-400',
    },
  };

  const c = config[level] || config.team_installable;

  return (
    <span className={`inline-flex items-center text-xs px-2 py-0.5 rounded ${c.className}`}>
      {c.label}
    </span>
  );
};

// Helper: Generate avatar initials
const getInitials = (name: string) => {
  return name.slice(0, 2).toUpperCase();
};

// Helper: Generate avatar color based on name
const getAvatarColor = (name: string) => {
  const colors = [
    'bg-blue-500', 'bg-green-500', 'bg-purple-500', 'bg-orange-500',
    'bg-pink-500', 'bg-teal-500', 'bg-indigo-500', 'bg-red-500'
  ];
  const index = name.charCodeAt(0) % colors.length;
  return colors[index];
};

// Bento style Members list component
const MembersListBento: React.FC<{
  members: TeamMember[];
  currentUserId?: string;
  isAdminOrOwner: boolean;
  onEditRole: (member: TeamMember) => void;
  onRemove: (member: TeamMember) => void;
  onAddMember: () => void;
}> = ({ members, currentUserId, isAdminOrOwner, onEditRole, onRemove, onAddMember }) => {
  const { t } = useTranslation('team');

  return (
    <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
      {members.map((member) => {
        const isOwner = member.role === 'owner';
        const isSelf = member.userId === currentUserId;
        const canManage = isAdminOrOwner && !isOwner && !isSelf;

        return (
          <div
            key={member.id}
            className="group relative p-4 rounded-xl border border-border-subtle bg-background-card hover:border-border-default transition-all"
          >
            <div className="flex items-start gap-3">
              {/* Avatar */}
              <div className={`w-10 h-10 rounded-full ${getAvatarColor(member.displayName)} flex items-center justify-center text-white text-sm font-medium`}>
                {getInitials(member.displayName)}
              </div>

              <div className="flex-1 min-w-0">
                <div className="flex items-center gap-2">
                  <p className="font-medium text-text-default truncate">{member.displayName}</p>
                  {member.status === 'active' && (
                    <div className="w-2 h-2 rounded-full bg-green-500" title={t('memberStatus.active')} />
                  )}
                </div>
                <p className="text-xs text-text-muted">{t(`roles.${member.role}`, member.role)}</p>
              </div>

              {canManage && (
                <DropdownMenu>
                  <DropdownMenuTrigger asChild>
                    <Button variant="ghost" size="sm" className="h-8 w-8 p-0 opacity-0 group-hover:opacity-100 transition-opacity">
                      <MoreVertical size={16} />
                    </Button>
                  </DropdownMenuTrigger>
                  <DropdownMenuContent align="end">
                    <DropdownMenuItem onClick={() => onEditRole(member)}>
                      <Pencil size={14} />
                      {t('memberManage.editRole')}
                    </DropdownMenuItem>
                    <DropdownMenuSeparator />
                    <DropdownMenuItem variant="destructive" onClick={() => onRemove(member)}>
                      <Trash2 size={14} />
                      {t('memberManage.remove')}
                    </DropdownMenuItem>
                  </DropdownMenuContent>
                </DropdownMenu>
              )}
            </div>
          </div>
        );
      })}

      {/* Add member card */}
      {isAdminOrOwner && (
        <button
          onClick={onAddMember}
          className="p-4 rounded-xl border border-dashed border-border-subtle hover:border-border-default hover:bg-background-muted/50 transition-all flex flex-col items-center justify-center gap-2 min-h-[88px]"
        >
          <UserPlus size={20} className="text-text-muted" />
          <span className="text-sm text-text-muted">{t('memberManage.addMember')}</span>
        </button>
      )}
    </div>
  );
};

// Bento style Skills list component
const SkillsListBento: React.FC<{
  skills: SharedSkill[];
  onInstall: (id: string) => void;
  installingId: string | null;
  canDelete: (authorId: string) => boolean;
  canEdit: (authorId: string) => boolean;
  onDelete: (skill: SharedSkill) => void;
  onViewDetail: (skill: SharedSkill) => void;
  onEdit: (skill: SharedSkill) => void;
  onShare: () => void;
}> = ({ skills, onInstall, installingId, canDelete, canEdit, onDelete, onViewDetail, onEdit, onShare }) => {
  const { t } = useTranslation('team');

  if (skills.length === 0) {
    return (
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
        {/* Empty state card */}
        <div className="col-span-full sm:col-span-1 p-6 rounded-xl border border-border-subtle bg-background-card text-center">
          <Sparkles size={32} className="mx-auto mb-3 text-text-muted" />
          <h3 className="font-medium text-text-default mb-1">{t('noSkills')}</h3>
          <p className="text-sm text-text-muted mb-4">{t('emptyState.skillsDesc')}</p>
        </div>
        {/* Share card */}
        <button
          onClick={onShare}
          className="p-6 rounded-xl border border-dashed border-border-subtle hover:border-border-default hover:bg-background-muted/50 transition-all flex flex-col items-center justify-center gap-2"
        >
          <Plus size={24} className="text-text-muted" />
          <span className="text-sm text-text-muted">{t('quickActions.shareSkill')}</span>
        </button>
      </div>
    );
  }

  return (
    <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
      {skills.map((skill) => {
        const showDeleteOption = canDelete(skill.authorId);
        const showEditOption = canEdit(skill.authorId);
        const canInstallLocally = allowsLocalInstall(skill.protectionLevel);

        return (
          <div
            key={skill.id}
            className="group relative p-4 rounded-xl border border-border-subtle bg-background-card hover:border-border-default transition-all"
          >
            <div className="flex items-start justify-between gap-3 mb-3">
              <div className="flex-1 min-w-0">
                <div className="flex items-center gap-2 mb-1">
                  <h3 className="font-medium text-text-default truncate">{skill.name}</h3>
                  <ProtectionLevelBadge level={skill.protectionLevel} />
                </div>
                {skill.description && (
                  <p className="text-sm text-text-muted line-clamp-2">{skill.description}</p>
                )}
              </div>
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <Button variant="ghost" size="sm" className="h-8 w-8 p-0 opacity-0 group-hover:opacity-100 transition-opacity">
                    <MoreVertical size={16} />
                  </Button>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="end">
                  <DropdownMenuItem onClick={() => onViewDetail(skill)}>
                    <Eye size={14} />
                    {t('manage.viewDetail')}
                  </DropdownMenuItem>
                  {showEditOption && (
                    <DropdownMenuItem onClick={() => onEdit(skill)}>
                      <Pencil size={14} />
                      {t('manage.edit')}
                    </DropdownMenuItem>
                  )}
                  {showDeleteOption && (
                    <>
                      <DropdownMenuSeparator />
                      <DropdownMenuItem variant="destructive" onClick={() => onDelete(skill)}>
                        <Trash2 size={14} />
                        {t('manage.delete')}
                      </DropdownMenuItem>
                    </>
                  )}
                </DropdownMenuContent>
              </DropdownMenu>
            </div>

            <div className="flex items-center justify-between">
              <span className="text-xs text-text-muted">v{skill.version}</span>
              {canInstallLocally ? (
                <Button size="sm" onClick={() => onInstall(skill.id)} disabled={installingId === skill.id} className="h-7 text-xs">
                  {installingId === skill.id ? (
                    <div className="animate-spin h-3 w-3 border-2 border-white border-t-transparent rounded-full" />
                  ) : (
                    <>
                      <Download size={12} className="mr-1" />
                      {t('install')}
                    </>
                  )}
                </Button>
              ) : (
                <span className="text-xs text-text-muted flex items-center gap-1">
                  <Cloud size={12} />
                  {t('onlineOnly')}
                </span>
              )}
            </div>
          </div>
        );
      })}

      {/* Share card */}
      <button
        onClick={onShare}
        className="p-4 rounded-xl border border-dashed border-border-subtle hover:border-border-default hover:bg-background-muted/50 transition-all flex flex-col items-center justify-center gap-2 min-h-[140px]"
      >
        <Plus size={20} className="text-text-muted" />
        <span className="text-sm text-text-muted">{t('quickActions.shareSkill')}</span>
      </button>
    </div>
  );
};

// Bento style Recipes list component
const RecipesListBento: React.FC<{
  recipes: SharedRecipe[];
  onInstall: (id: string) => void;
  installingId: string | null;
  canDelete: (authorId: string) => boolean;
  canEdit: (authorId: string) => boolean;
  onDelete: (recipe: SharedRecipe) => void;
  onViewDetail: (recipe: SharedRecipe) => void;
  onEdit: (recipe: SharedRecipe) => void;
  onShare: () => void;
}> = ({ recipes, onInstall, installingId, canDelete, canEdit, onDelete, onViewDetail, onEdit, onShare }) => {
  const { t } = useTranslation('team');

  if (recipes.length === 0) {
    return (
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
        <div className="col-span-full sm:col-span-1 p-6 rounded-xl border border-border-subtle bg-background-card text-center">
          <Book size={32} className="mx-auto mb-3 text-text-muted" />
          <h3 className="font-medium text-text-default mb-1">{t('noRecipes')}</h3>
          <p className="text-sm text-text-muted mb-4">{t('emptyState.recipesDesc')}</p>
        </div>
        <button
          onClick={onShare}
          className="p-6 rounded-xl border border-dashed border-border-subtle hover:border-border-default hover:bg-background-muted/50 transition-all flex flex-col items-center justify-center gap-2"
        >
          <Plus size={24} className="text-text-muted" />
          <span className="text-sm text-text-muted">{t('quickActions.shareRecipe')}</span>
        </button>
      </div>
    );
  }

  return (
    <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
      {recipes.map((recipe) => {
        const showDeleteOption = canDelete(recipe.authorId);
        const showEditOption = canEdit(recipe.authorId);
        const canInstallLocally = allowsLocalInstall(recipe.protectionLevel);

        return (
          <div
            key={recipe.id}
            className="group relative p-4 rounded-xl border border-border-subtle bg-background-card hover:border-border-default transition-all"
          >
            <div className="flex items-start justify-between gap-3 mb-3">
              <div className="flex-1 min-w-0">
                <div className="flex items-center gap-2 mb-1">
                  <h3 className="font-medium text-text-default truncate">{recipe.name}</h3>
                  <ProtectionLevelBadge level={recipe.protectionLevel} />
                </div>
                {recipe.description && (
                  <p className="text-sm text-text-muted line-clamp-2">{recipe.description}</p>
                )}
              </div>
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <Button variant="ghost" size="sm" className="h-8 w-8 p-0 opacity-0 group-hover:opacity-100 transition-opacity">
                    <MoreVertical size={16} />
                  </Button>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="end">
                  <DropdownMenuItem onClick={() => onViewDetail(recipe)}>
                    <Eye size={14} />
                    {t('manage.viewDetail')}
                  </DropdownMenuItem>
                  {showEditOption && (
                    <DropdownMenuItem onClick={() => onEdit(recipe)}>
                      <Pencil size={14} />
                      {t('manage.edit')}
                    </DropdownMenuItem>
                  )}
                  {showDeleteOption && (
                    <>
                      <DropdownMenuSeparator />
                      <DropdownMenuItem variant="destructive" onClick={() => onDelete(recipe)}>
                        <Trash2 size={14} />
                        {t('manage.delete')}
                      </DropdownMenuItem>
                    </>
                  )}
                </DropdownMenuContent>
              </DropdownMenu>
            </div>

            <div className="flex items-center justify-between">
              <span className="text-xs text-text-muted">v{recipe.version}</span>
              {canInstallLocally ? (
                <Button size="sm" onClick={() => onInstall(recipe.id)} disabled={installingId === recipe.id} className="h-7 text-xs">
                  {installingId === recipe.id ? (
                    <div className="animate-spin h-3 w-3 border-2 border-white border-t-transparent rounded-full" />
                  ) : (
                    <>
                      <Download size={12} className="mr-1" />
                      {t('install')}
                    </>
                  )}
                </Button>
              ) : (
                <span className="text-xs text-text-muted flex items-center gap-1">
                  <Cloud size={12} />
                  {t('onlineOnly')}
                </span>
              )}
            </div>
          </div>
        );
      })}

      <button
        onClick={onShare}
        className="p-4 rounded-xl border border-dashed border-border-subtle hover:border-border-default hover:bg-background-muted/50 transition-all flex flex-col items-center justify-center gap-2 min-h-[140px]"
      >
        <Plus size={20} className="text-text-muted" />
        <span className="text-sm text-text-muted">{t('quickActions.shareRecipe')}</span>
      </button>
    </div>
  );
};

// Bento style Extensions list component
const ExtensionsListBento: React.FC<{
  extensions: SharedExtension[];
  onInstall: (id: string) => void;
  installingId: string | null;
  canDelete: (authorId: string) => boolean;
  canEdit: (authorId: string) => boolean;
  onDelete: (ext: SharedExtension) => void;
  onViewDetail: (ext: SharedExtension) => void;
  onEdit: (ext: SharedExtension) => void;
  canReview: boolean;
  onReview: (ext: SharedExtension, approved: boolean) => void;
  onShare: () => void;
}> = ({ extensions, onInstall, installingId, canDelete, canEdit, onDelete, onViewDetail, onEdit, canReview, onReview, onShare }) => {
  const { t } = useTranslation('team');

  if (extensions.length === 0) {
    return (
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
        <div className="col-span-full sm:col-span-1 p-6 rounded-xl border border-border-subtle bg-background-card text-center">
          <Puzzle size={32} className="mx-auto mb-3 text-text-muted" />
          <h3 className="font-medium text-text-default mb-1">{t('noExtensions')}</h3>
          <p className="text-sm text-text-muted mb-4">{t('emptyState.extensionsDesc')}</p>
        </div>
        <button
          onClick={onShare}
          className="p-6 rounded-xl border border-dashed border-border-subtle hover:border-border-default hover:bg-background-muted/50 transition-all flex flex-col items-center justify-center gap-2"
        >
          <Plus size={24} className="text-text-muted" />
          <span className="text-sm text-text-muted">{t('quickActions.shareExtension')}</span>
        </button>
      </div>
    );
  }

  return (
    <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
      {extensions.map((ext) => {
        const isBuiltin = isBuiltinExtension(ext);
        const showDeleteOption = canDelete(ext.authorId) && !isBuiltin;
        const showEditOption = canEdit(ext.authorId) && !isBuiltin;
        const showReviewOptions = canReview && !ext.securityReviewed && !isBuiltin;
        const canInstallLocally = allowsLocalInstall(ext.protectionLevel);

        return (
          <div
            key={ext.id}
            className="group relative p-4 rounded-xl border border-border-subtle bg-background-card hover:border-border-default transition-all"
          >
            <div className="flex items-start justify-between gap-3 mb-3">
              <div className="flex-1 min-w-0">
                <div className="flex items-center gap-2 mb-1">
                  <h3 className="font-medium text-text-default truncate">{ext.name}</h3>
                  <ProtectionLevelBadge level={ext.protectionLevel} />
                  {isBuiltin && (
                    <span className="inline-flex items-center gap-1 text-xs px-2 py-0.5 rounded bg-purple-500/10 text-purple-600 dark:text-purple-400">
                      <Lock size={10} />
                      {t('extensionType.builtin', '内置')}
                    </span>
                  )}
                  {!ext.securityReviewed && (
                    <AlertTriangle size={12} className="text-amber-500" />
                  )}
                  {ext.securityReviewed && (
                    <Check size={12} className="text-green-500" />
                  )}
                </div>
                {ext.description && (
                  <p className="text-sm text-text-muted line-clamp-2">{ext.description}</p>
                )}
              </div>
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <Button variant="ghost" size="sm" className="h-8 w-8 p-0 opacity-0 group-hover:opacity-100 transition-opacity">
                    <MoreVertical size={16} />
                  </Button>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="end">
                  <DropdownMenuItem onClick={() => onViewDetail(ext)}>
                    <Eye size={14} />
                    {t('manage.viewDetail')}
                  </DropdownMenuItem>
                  {showEditOption && (
                    <DropdownMenuItem onClick={() => onEdit(ext)}>
                      <Pencil size={14} />
                      {t('manage.edit')}
                    </DropdownMenuItem>
                  )}
                  {showReviewOptions && (
                    <>
                      <DropdownMenuSeparator />
                      <DropdownMenuItem onClick={() => onReview(ext, true)}>
                        <CheckCircle size={14} className="text-green-600" />
                        {t('manage.approve')}
                      </DropdownMenuItem>
                      <DropdownMenuItem onClick={() => onReview(ext, false)}>
                        <XCircle size={14} className="text-red-600" />
                        {t('manage.reject')}
                      </DropdownMenuItem>
                    </>
                  )}
                  {showDeleteOption && (
                    <>
                      <DropdownMenuSeparator />
                      <DropdownMenuItem variant="destructive" onClick={() => onDelete(ext)}>
                        <Trash2 size={14} />
                        {t('manage.delete')}
                      </DropdownMenuItem>
                    </>
                  )}
                </DropdownMenuContent>
              </DropdownMenu>
            </div>

            <div className="flex items-center justify-between">
              <div className="flex items-center gap-2 text-xs text-text-muted">
                <span>v{ext.version}</span>
                <span className="bg-background-muted px-1.5 py-0.5 rounded">{ext.extensionType}</span>
              </div>
              {canInstallLocally ? (
                <Button size="sm" onClick={() => onInstall(ext.id)} disabled={installingId === ext.id} className="h-7 text-xs">
                  {installingId === ext.id ? (
                    <div className="animate-spin h-3 w-3 border-2 border-white border-t-transparent rounded-full" />
                  ) : (
                    <>
                      <Download size={12} className="mr-1" />
                      {t('install')}
                    </>
                  )}
                </Button>
              ) : (
                <span className="text-xs text-text-muted flex items-center gap-1">
                  <Cloud size={12} />
                  {t('onlineOnly')}
                </span>
              )}
            </div>
          </div>
        );
      })}

      <button
        onClick={onShare}
        className="p-4 rounded-xl border border-dashed border-border-subtle hover:border-border-default hover:bg-background-muted/50 transition-all flex flex-col items-center justify-center gap-2 min-h-[140px]"
      >
        <Plus size={20} className="text-text-muted" />
        <span className="text-sm text-text-muted">{t('quickActions.shareExtension')}</span>
      </button>
    </div>
  );
};

export default TeamDetail;
