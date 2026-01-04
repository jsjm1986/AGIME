import React from 'react';
import { useTranslation } from 'react-i18next';
import i18n from '../../i18n';
import { Card } from '../ui/card';
import { formatDate } from '../../utils/date';
import { Session } from '../../api';

interface SessionItemProps {
  session: Session;
  extraActions?: React.ReactNode;
}

const SessionItem: React.FC<SessionItemProps> = ({ session, extraActions }) => {
  const { t } = useTranslation('sessions');

  return (
    <Card className="p-4 mb-2 hover:bg-accent/50 cursor-pointer flex justify-between items-center">
      <div>
        <div className="font-medium">{session.name}</div>
        <div className="text-sm text-muted-foreground">
          {formatDate(session.updated_at, i18n.language)} • {t('messages', { count: session.message_count })}
        </div>
        <div className="text-sm text-muted-foreground">{session.working_dir}</div>
      </div>
      {extraActions && <div>{extraActions}</div>}
    </Card>
  );
};

export default SessionItem;
