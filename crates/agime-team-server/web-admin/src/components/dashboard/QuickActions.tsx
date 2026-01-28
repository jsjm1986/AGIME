import { Link } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import { Card, CardHeader, CardTitle, CardContent } from '../ui/card';
import { Button } from '../ui/button';

export function QuickActions() {
  const { t } = useTranslation();

  return (
    <Card>
      <CardHeader>
        <CardTitle>{t('dashboard.quickActions')}</CardTitle>
      </CardHeader>
      <CardContent className="flex flex-wrap gap-2">
        <Link to="/teams">
          <Button>{t('dashboard.manageTeams')}</Button>
        </Link>
        <Link to="/api-keys">
          <Button variant="outline">{t('dashboard.manageApiKeys')}</Button>
        </Link>
      </CardContent>
    </Card>
  );
}
