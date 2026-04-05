import { useState, useEffect, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { Plus, Trash2, Users, UserPlus, UserMinus } from 'lucide-react';
import { Button } from '../ui/button';
import { Input } from '../ui/input';
import { Card, CardContent, CardHeader, CardTitle } from '../ui/card';
import { Badge } from '../ui/badge';
import { ConfirmDialog } from '../ui/confirm-dialog';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '../ui/select';
import { apiClient } from '../../api/client';
import { userGroupApi } from '../../api/userGroups';
import type { UserGroupSummary, UserGroupDetail } from '../../api/userGroups';
import type { TeamMember } from '../../api/types';
import { useToast } from '../../contexts/ToastContext';

interface Props {
  teamId: string;
}

export function UserGroupsTab({ teamId }: Props) {
  const { t } = useTranslation();
  const { addToast } = useToast();
  const [groups, setGroups] = useState<UserGroupSummary[]>([]);
  const [teamMembers, setTeamMembers] = useState<TeamMember[]>([]);
  const [loading, setLoading] = useState(true);
  const [membersLoading, setMembersLoading] = useState(true);
  const [selectedGroup, setSelectedGroup] = useState<UserGroupDetail | null>(null);
  const [showCreate, setShowCreate] = useState(false);
  const [newName, setNewName] = useState('');
  const [newDesc, setNewDesc] = useState('');
  const [creating, setCreating] = useState(false);
  const [selectedMemberId, setSelectedMemberId] = useState('');
  const [deleteTarget, setDeleteTarget] = useState<string | null>(null);

  const loadGroups = useCallback(async () => {
    try {
      setLoading(true);
      const res = await userGroupApi.list(teamId);
      setGroups(res.items);
    } catch (err) {
      console.error('Failed to load groups:', err);
      addToast('error', err instanceof Error ? err.message : t('common.error'));
    } finally {
      setLoading(false);
    }
  }, [addToast, t, teamId]);

  const loadTeamMembers = useCallback(async () => {
    try {
      setMembersLoading(true);
      const res = await apiClient.getMembers(teamId);
      setTeamMembers(res.members);
    } catch (err) {
      console.error('Failed to load team members:', err);
      addToast('error', err instanceof Error ? err.message : t('common.error'));
    } finally {
      setMembersLoading(false);
    }
  }, [addToast, t, teamId]);

  useEffect(() => {
    loadGroups();
    loadTeamMembers();
  }, [loadGroups, loadTeamMembers]);

  const handleCreate = async () => {
    if (!newName.trim()) return;
    try {
      setCreating(true);
      await userGroupApi.create(teamId, {
        name: newName.trim(),
        description: newDesc.trim() || undefined,
      });
      setNewName('');
      setNewDesc('');
      setShowCreate(false);
      loadGroups();
    } catch (err) {
      console.error('Failed to create group:', err);
      addToast('error', err instanceof Error ? err.message : t('common.error'));
    } finally {
      setCreating(false);
    }
  };

  const handleDelete = (groupId: string) => {
    setDeleteTarget(groupId);
  };

  const confirmDelete = async () => {
    if (!deleteTarget) return;
    try {
      await userGroupApi.delete(teamId, deleteTarget);
      if (selectedGroup?.id === deleteTarget) setSelectedGroup(null);
      loadGroups();
    } catch (err) {
      console.error('Failed to delete group:', err);
      addToast('error', err instanceof Error ? err.message : t('common.error'));
    } finally {
      setDeleteTarget(null);
    }
  };

  const handleSelectGroup = async (groupId: string) => {
    try {
      const detail = await userGroupApi.get(teamId, groupId);
      setSelectedGroup(detail);
      setSelectedMemberId('');
    } catch (err) {
      console.error('Failed to load group detail:', err);
      addToast('error', err instanceof Error ? err.message : t('common.error'));
    }
  };

  const handleAddMember = async () => {
    if (!selectedGroup || !selectedMemberId) return;
    try {
      const updated = await userGroupApi.updateMembers(
        teamId,
        selectedGroup.id,
        { add: [selectedMemberId] },
      );
      setSelectedGroup(updated);
      setSelectedMemberId('');
      loadGroups();
    } catch (err) {
      console.error('Failed to add member:', err);
      addToast('error', err instanceof Error ? err.message : t('common.error'));
    }
  };

  const handleRemoveMember = async (userId: string) => {
    if (!selectedGroup) return;
    try {
      const updated = await userGroupApi.updateMembers(
        teamId, selectedGroup.id, { remove: [userId] }
      );
      setSelectedGroup(updated);
      loadGroups();
    } catch (err) {
      console.error('Failed to remove member:', err);
      addToast('error', err instanceof Error ? err.message : t('common.error'));
    }
  };

  const availableMembers = selectedGroup
    ? teamMembers.filter((member) => !selectedGroup.members.includes(member.userId))
    : [];

  return (
    <>
    <div className="space-y-4">
      {/* Header */}
      <div className="flex items-center justify-between">
        <h3 className="text-lg font-medium flex items-center gap-2">
          <Users className="h-5 w-5" />
          {t('userGroups.title')}
        </h3>
        <Button size="sm" onClick={() => setShowCreate(!showCreate)}>
          <Plus className="h-4 w-4 mr-1" />
          {t('userGroups.create')}
        </Button>
      </div>

      {/* Create Form */}
      {showCreate && (
        <Card>
          <CardContent className="pt-4 space-y-3">
            <Input
              placeholder={t('userGroups.namePlaceholder')}
              value={newName}
              onChange={(e) => setNewName(e.target.value)}
            />
            <Input
              placeholder={t('userGroups.descriptionPlaceholder')}
              value={newDesc}
              onChange={(e) => setNewDesc(e.target.value)}
            />
            <div className="flex gap-2">
              <Button size="sm" onClick={handleCreate} disabled={creating || !newName.trim()}>
                {creating ? t('common.creating') : t('common.create')}
              </Button>
              <Button size="sm" variant="outline" onClick={() => setShowCreate(false)}>
                {t('common.cancel')}
              </Button>
            </div>
          </CardContent>
        </Card>
      )}

      {/* Groups List + Detail */}
      <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
        {/* Groups List */}
        <div className="space-y-2">
          {loading ? (
            <p className="text-sm text-muted-foreground">{t('common.loading')}</p>
          ) : groups.length === 0 ? (
            <p className="text-sm text-muted-foreground">{t('userGroups.noGroups')}</p>
          ) : (
            groups.map((g) => (
              <Card
                key={g.id}
                className={`cursor-pointer transition-colors ${
                  selectedGroup?.id === g.id ? 'border-primary' : ''
                }`}
                onClick={() => handleSelectGroup(g.id)}
              >
                <CardContent className="py-3 flex items-center justify-between">
                  <div>
                    <div className="font-medium flex items-center gap-2">
                      {g.name}
                      {g.isSystem && (
                        <Badge variant="secondary">{t('userGroups.system')}</Badge>
                      )}
                    </div>
                    <p className="text-xs text-muted-foreground">
                      {t('userGroups.memberCount', { count: g.memberCount })}
                    </p>
                  </div>
                  {!g.isSystem && (
                    <Button
                      size="sm"
                      variant="ghost"
                      onClick={(e) => { e.stopPropagation(); handleDelete(g.id); }}
                    >
                      <Trash2 className="h-4 w-4" />
                    </Button>
                  )}
                </CardContent>
              </Card>
            ))
          )}
        </div>

        {/* Group Detail */}
        {selectedGroup && (
            <Card>
              <CardHeader>
                <CardTitle className="text-base">{selectedGroup.name}</CardTitle>
              {selectedGroup.description && (
                <p className="text-sm text-muted-foreground">{selectedGroup.description}</p>
              )}
              </CardHeader>
              <CardContent className="space-y-3">
                <div className="flex items-center gap-2">
                  <Select
                    value={selectedMemberId}
                    onValueChange={setSelectedMemberId}
                    disabled={membersLoading || availableMembers.length === 0}
                  >
                    <SelectTrigger className="flex-1">
                      <SelectValue
                        placeholder={
                          membersLoading
                            ? t('common.loading')
                            : t('userGroups.memberSelectPlaceholder')
                        }
                      />
                    </SelectTrigger>
                    <SelectContent>
                      {availableMembers.map((member) => (
                        <SelectItem key={member.userId} value={member.userId}>
                          {member.displayName || member.userId}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                  <Button size="sm" onClick={handleAddMember} disabled={!selectedMemberId}>
                    <UserPlus className="h-4 w-4" />
                  </Button>
                </div>
                {!membersLoading && availableMembers.length === 0 && (
                  <p className="text-xs text-muted-foreground">
                    {t('userGroups.noAvailableMembers')}
                  </p>
                )}
                <div className="space-y-1">
                  <p className="text-sm font-medium">
                    {t('userGroups.members')} ({selectedGroup.members.length})
                  </p>
                  {selectedGroup.members.length === 0 ? (
                    <p className="text-xs text-muted-foreground">{t('userGroups.noMembers')}</p>
                  ) : (
                    selectedGroup.memberDetails.map((member) => (
                      <div
                        key={member.userId}
                        className="flex items-center justify-between rounded px-2 py-2 hover:bg-muted"
                      >
                        <div className="min-w-0">
                          <p className="truncate text-sm font-medium">
                            {member.displayName || member.userId}
                          </p>
                          <p className="truncate text-xs text-muted-foreground">
                            {member.email || member.userId}
                          </p>
                        </div>
                        <Button
                          size="sm"
                          variant="ghost"
                          onClick={() => handleRemoveMember(member.userId)}
                        >
                          <UserMinus className="h-3 w-3" />
                        </Button>
                      </div>
                    ))
                  )}
                </div>
              </CardContent>
          </Card>
        )}
      </div>
    </div>
    <ConfirmDialog
      open={!!deleteTarget}
      onOpenChange={(open) => { if (!open) setDeleteTarget(null); }}
      title={t('userGroups.deleteConfirm')}
      variant="destructive"
      onConfirm={confirmDelete}
    />
    </>
  );
}
