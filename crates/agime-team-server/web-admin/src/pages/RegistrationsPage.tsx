import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { AppShell } from '../components/layout/AppShell';
import { PageHeader } from '../components/layout/PageHeader';
import { Card, CardHeader, CardTitle, CardContent } from '../components/ui/card';
import { Button } from '../components/ui/button';
import { apiClient } from '../api/client';
import { useToast } from '../contexts/ToastContext';

interface RegistrationRequest {
  request_id: string;
  email: string;
  display_name: string;
  status: string;
  created_at: string;
}

export function RegistrationsPage() {
  const { t } = useTranslation();
  const { addToast } = useToast();
  const [requests, setRequests] = useState<RegistrationRequest[]>([]);
  const [loading, setLoading] = useState(true);

  const fetchRequests = async () => {
    try {
      const res = await apiClient.getRegistrations();
      setRequests(res.requests);
    } catch {
      addToast('error', t('common.error'));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => { fetchRequests(); }, []);

  const handleApprove = async (id: string) => {
    try {
      await apiClient.approveRegistration(id);
      addToast('success', t('registrations.approved'));
      fetchRequests();
    } catch {
      addToast('error', t('common.error'));
    }
  };

  const handleReject = async (id: string) => {
    try {
      await apiClient.rejectRegistration(id);
      addToast('success', t('registrations.rejected'));
      fetchRequests();
    } catch {
      addToast('error', t('common.error'));
    }
  };

  return (
    <AppShell>
      <PageHeader title={t('registrations.title')} />
      <div className="p-6">
        <Card>
          <CardHeader>
            <CardTitle>{t('registrations.pendingRequests')}</CardTitle>
          </CardHeader>
          <CardContent>
            {loading ? (
              <p className="text-sm text-[hsl(var(--muted-foreground))]">{t('common.loading')}</p>
            ) : requests.length === 0 ? (
              <p className="text-sm text-[hsl(var(--muted-foreground))]">{t('registrations.noRequests')}</p>
            ) : (
              <div className="space-y-3">
                {requests.map((req) => (
                  <div key={req.request_id} className="flex items-center justify-between p-3 rounded-md border border-[hsl(var(--border))]">
                    <div>
                      <p className="text-sm font-medium">{req.display_name}</p>
                      <p className="text-xs text-[hsl(var(--muted-foreground))]">{req.email}</p>
                      <p className="text-xs text-[hsl(var(--muted-foreground))]">
                        {new Date(req.created_at).toLocaleString()}
                      </p>
                    </div>
                    <div className="flex gap-2">
                      <Button size="sm" onClick={() => handleApprove(req.request_id)}>
                        {t('registrations.approve')}
                      </Button>
                      <Button size="sm" variant="outline" onClick={() => handleReject(req.request_id)}>
                        {t('registrations.reject')}
                      </Button>
                    </div>
                  </div>
                ))}
              </div>
            )}
          </CardContent>
        </Card>
      </div>
    </AppShell>
  );
}
