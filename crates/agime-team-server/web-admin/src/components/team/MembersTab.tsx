import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { UserMinus, Shield } from 'lucide-react';
import { Button } from '../ui/button';
import { Badge } from '../ui/badge';
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '../ui/table';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '../ui/select';
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from '../ui/dialog';
import { apiClient } from '../../api/client';
import type { TeamMember, TeamRole } from '../../api/types';

interface MembersTabProps {
  teamId: string;
  userRole: TeamRole;
  onUpdate: () => void;
}

export function MembersTab({ teamId, userRole, onUpdate }: MembersTabProps) {
  const { t } = useTranslation();
  const [members, setMembers] = useState<TeamMember[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [editingMember, setEditingMember] = useState<TeamMember | null>(null);
  const [removingMember, setRemovingMember] = useState<TeamMember | null>(null);
  const [newRole, setNewRole] = useState<TeamRole>('member');
  const [actionLoading, setActionLoading] = useState(false);

  const canManage = userRole === 'owner' || userRole === 'admin';

  const loadMembers = async () => {
    try {
      setLoading(true);
      const response = await apiClient.getMembers(teamId);
      setMembers(response.members);
      setError('');
    } catch (err) {
      setError(err instanceof Error ? err.message : t('common.error'));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    loadMembers();
  }, [teamId]);

  const handleUpdateRole = async () => {
    if (!editingMember) return;
    setActionLoading(true);
    try {
      await apiClient.updateMember(editingMember.id, newRole);
      setEditingMember(null);
      loadMembers();
      onUpdate();
    } catch (err) {
      setError(err instanceof Error ? err.message : t('common.error'));
    } finally {
      setActionLoading(false);
    }
  };

  const handleRemove = async () => {
    if (!removingMember) return;
    setActionLoading(true);
    try {
      await apiClient.removeMember(removingMember.id);
      setRemovingMember(null);
      loadMembers();
      onUpdate();
    } catch (err) {
      setError(err instanceof Error ? err.message : t('common.error'));
    } finally {
      setActionLoading(false);
    }
  };

  const roleVariant = {
    owner: 'default' as const,
    admin: 'secondary' as const,
    member: 'outline' as const,
  };

  if (loading) {
    return <p className="text-center py-8 text-[hsl(var(--muted-foreground))]">{t('common.loading')}</p>;
  }

  if (error) {
    return <p className="text-center py-8 text-[hsl(var(--destructive))]">{error}</p>;
  }

  return (
    <>
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>{t('teams.member.name')}</TableHead>
            <TableHead>{t('teams.member.userId')}</TableHead>
            <TableHead>{t('teams.member.role')}</TableHead>
            <TableHead>{t('teams.member.joinedAt')}</TableHead>
            {canManage && <TableHead className="w-[100px]">{t('common.actions')}</TableHead>}
          </TableRow>
        </TableHeader>
        <TableBody>
          {members.map((member) => (
            <TableRow key={member.id}>
              <TableCell className="font-medium">{member.displayName}</TableCell>
              <TableCell>{member.userId}</TableCell>
              <TableCell>
                <Badge variant={roleVariant[member.role]}>
                  {t(`teams.roles.${member.role}`)}
                </Badge>
              </TableCell>
              <TableCell>{new Date(member.joinedAt).toLocaleDateString()}</TableCell>
              {canManage && (
                <TableCell>
                  {member.role !== 'owner' && (
                    <div className="flex gap-1">
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => {
                          setEditingMember(member);
                          setNewRole(member.role);
                        }}
                      >
                        <Shield className="h-4 w-4" />
                      </Button>
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => setRemovingMember(member)}
                      >
                        <UserMinus className="h-4 w-4" />
                      </Button>
                    </div>
                  )}
                </TableCell>
              )}
            </TableRow>
          ))}
        </TableBody>
      </Table>

      {/* Edit Role Dialog */}
      <Dialog open={!!editingMember} onOpenChange={() => setEditingMember(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t('teams.member.editRole')}</DialogTitle>
          </DialogHeader>
          <div className="py-4">
            <p className="mb-4">{editingMember?.displayName}</p>
            <Select value={newRole} onValueChange={(v) => setNewRole(v as TeamRole)}>
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="admin">{t('teams.roles.admin')}</SelectItem>
                <SelectItem value="member">{t('teams.roles.member')}</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setEditingMember(null)}>
              {t('common.cancel')}
            </Button>
            <Button onClick={handleUpdateRole} disabled={actionLoading}>
              {actionLoading ? t('common.saving') : t('common.save')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Remove Confirm Dialog */}
      <Dialog open={!!removingMember} onOpenChange={() => setRemovingMember(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t('teams.member.removeConfirm')}</DialogTitle>
          </DialogHeader>
          <p className="py-4">
            {t('teams.member.removeMessage', { name: removingMember?.displayName })}
          </p>
          <DialogFooter>
            <Button variant="outline" onClick={() => setRemovingMember(null)}>
              {t('common.cancel')}
            </Button>
            <Button variant="destructive" onClick={handleRemove} disabled={actionLoading}>
              {actionLoading ? t('common.removing') : t('common.remove')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}
