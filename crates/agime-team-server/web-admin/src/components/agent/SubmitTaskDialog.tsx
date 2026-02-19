import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from '../ui/dialog';
import { Button } from '../ui/button';
import { Label } from '../ui/label';
import { Textarea } from '../ui/textarea';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '../ui/select';
import { taskApi, SubmitTaskRequest, TeamAgent } from '../../api/agent';

interface Props {
  teamId: string;
  agents: TeamAgent[];
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSubmitted: () => void;
}

export function SubmitTaskDialog({ teamId, agents, open, onOpenChange, onSubmitted }: Props) {
  const { t } = useTranslation();
  const [loading, setLoading] = useState(false);
  const [agentId, setAgentId] = useState('');
  const [taskType, setTaskType] = useState<'chat' | 'recipe' | 'skill'>('chat');
  const [message, setMessage] = useState('');
  const [priority, setPriority] = useState(0);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!agentId || !message.trim()) return;

    setLoading(true);
    try {
      const req: SubmitTaskRequest = {
        team_id: teamId,
        agent_id: agentId,
        task_type: taskType,
        content: {
          messages: [{ role: 'user', content: message.trim() }],
        },
        priority,
      };
      await taskApi.submitTask(req);
      onSubmitted();
      onOpenChange(false);
      setMessage('');
      setPriority(0);
    } catch (error) {
      console.error('Failed to submit task:', error);
    } finally {
      setLoading(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[600px]">
        <DialogHeader>
          <DialogTitle>{t('agent.task.submit', 'Submit Task')}</DialogTitle>
        </DialogHeader>
        <form onSubmit={handleSubmit}>
          <div className="space-y-4 py-4">
            <div className="space-y-2">
              <Label>{t('agent.task.selectAgent', 'Select Agent')} *</Label>
              <Select value={agentId} onValueChange={setAgentId}>
                <SelectTrigger>
                  <SelectValue placeholder={t('agent.task.selectAgentPlaceholder', 'Choose an agent')} />
                </SelectTrigger>
                <SelectContent>
                  {agents.map((agent) => (
                    <SelectItem key={agent.id} value={agent.id}>
                      {agent.name} ({agent.api_format})
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>

            <div className="space-y-2">
              <Label>{t('agent.task.type', 'Task Type')}</Label>
              <Select value={taskType} onValueChange={(v) => setTaskType(v as typeof taskType)}>
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="chat">{t('agent.task.typeChat', 'Chat')}</SelectItem>
                  <SelectItem value="recipe">{t('agent.task.typeRecipe', 'Recipe')}</SelectItem>
                  <SelectItem value="skill">{t('agent.task.typeSkill', 'Skill')}</SelectItem>
                </SelectContent>
              </Select>
            </div>

            <div className="space-y-2">
              <Label htmlFor="message">{t('agent.task.message', 'Message')} *</Label>
              <Textarea
                id="message"
                value={message}
                onChange={(e) => setMessage(e.target.value)}
                placeholder={t('agent.task.messagePlaceholder', 'Enter your instruction...')}
                rows={6}
                required
              />
            </div>

            <div className="space-y-2">
              <Label>{t('agent.task.priority', 'Priority')}</Label>
              <Select value={String(priority)} onValueChange={(v) => setPriority(Number(v))}>
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="0">{t('agent.task.priorityNormal', 'Normal (0)')}</SelectItem>
                  <SelectItem value="25">{t('agent.task.priorityLow', 'Low (25)')}</SelectItem>
                  <SelectItem value="50">{t('agent.task.priorityMedium', 'Medium (50)')}</SelectItem>
                  <SelectItem value="75">{t('agent.task.priorityHigh', 'High (75)')}</SelectItem>
                  <SelectItem value="100">{t('agent.task.priorityUrgent', 'Urgent (100)')}</SelectItem>
                </SelectContent>
              </Select>
            </div>
          </div>

          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              {t('common.cancel')}
            </Button>
            <Button type="submit" disabled={loading || !agentId || !message.trim()}>
              {loading ? t('agent.task.submitting', 'Submitting...') : t('agent.task.submit', 'Submit')}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
