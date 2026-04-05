import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { AppShell } from '../components/layout/AppShell';
import { PageHeader } from '../components/layout/PageHeader';
import { StatsCard, QuickActions } from '../components/dashboard';
import { Card, CardHeader, CardTitle, CardContent } from '../components/ui/card';
import { Skeleton } from '../components/ui/skeleton';
import { useAuth } from '../contexts/AuthContext';
import { apiClient } from '../api/client';
import { copyText } from '../utils/clipboard';

const SERVER_URL = window.location.origin;

interface Stats {
  teamsCount: number;
  apiKeysCount: number;
}

export function DashboardPage() {
  const { t } = useTranslation();
  const { user } = useAuth();
  const [stats, setStats] = useState<Stats | null>(null);
  const [loading, setLoading] = useState(true);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    loadStats();
  }, []);

  const loadStats = async () => {
    try {
      const [teamsRes, keysRes] = await Promise.all([
        apiClient.getTeams(),
        apiClient.getApiKeys(),
      ]);
      setStats({
        teamsCount: teamsRes.total || teamsRes.teams?.length || 0,
        apiKeysCount: keysRes.keys?.length || 0,
      });
    } catch (error) {
      console.error('Failed to load stats:', error);
    } finally {
      setLoading(false);
    }
  };

  const handleCopyUrl = async () => {
    if (await copyText(SERVER_URL)) {
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    }
  };

  return (
    <AppShell>
      <PageHeader
        title={t('dashboard.title')}
        description={t('dashboard.welcome', { name: user?.display_name })}
      />

      {/* Stats Grid */}
      <div className="mb-6 grid grid-cols-1 gap-4 md:grid-cols-2 lg:grid-cols-4">
        {loading ? (
          <>
            <Skeleton className="h-24" />
            <Skeleton className="h-24" />
          </>
        ) : (
          <>
            <StatsCard
              title={t('stats.teams')}
              value={stats?.teamsCount || 0}
              icon={<TeamsIcon />}
            />
            <StatsCard
              title={t('stats.apiKeys')}
              value={stats?.apiKeysCount || 0}
              icon={<KeyIcon />}
            />
          </>
        )}
      </div>

      {/* Quick Actions */}
      <div className="mb-6">
        <QuickActions />
      </div>

      {/* Connection Guide */}
      <Card className="ui-section-panel">
        <CardHeader>
          <CardTitle className="ui-heading text-[24px]">{t('guide.title')}</CardTitle>
        </CardHeader>
        <CardContent className="space-y-4">
          <GuideStep number={1} title={t('guide.step1Title')}>
            <p className="ui-secondary-text">
              {t('guide.step1Desc')}
            </p>
          </GuideStep>

          <GuideStep number={2} title={t('guide.step2Title')}>
            <p className="ui-secondary-text mb-3">
              {t('guide.step2Desc')}
            </p>
            <div className="ui-copy-block p-3">
              <div className="flex items-center justify-between">
                <code className="text-sm">{SERVER_URL}</code>
                <button
                  onClick={handleCopyUrl}
                  className="ui-inline-action text-sm"
                >
                  {copied ? t('guide.copied') : t('guide.copyUrl')}
                </button>
              </div>
            </div>
          </GuideStep>

          <GuideStep number={3} title={t('guide.step3Title')}>
            <ul className="ui-secondary-text list-disc list-inside space-y-1">
              <li>{t('guide.feature1')}</li>
              <li>{t('guide.feature2')}</li>
              <li>{t('guide.feature3')}</li>
            </ul>
          </GuideStep>
        </CardContent>
      </Card>
    </AppShell>
  );
}

function GuideStep({ number, title, children }: {
  number: number;
  title: string;
  children: React.ReactNode;
}) {
  return (
    <div className="ui-subtle-panel flex gap-4 p-4 sm:p-5">
      <div className="flex h-9 w-9 flex-shrink-0 items-center justify-center rounded-full bg-[hsl(var(--primary))] text-[hsl(var(--primary-foreground))]">
        {number}
      </div>
      <div className="flex-1">
        <h3 className="mb-2 text-sm font-semibold text-[hsl(var(--foreground))]">{title}</h3>
        {children}
      </div>
    </div>
  );
}

function TeamsIcon() {
  return (
    <svg className="w-6 h-6" fill="none" viewBox="0 0 24 24" stroke="currentColor">
      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2}
        d="M17 20h5v-2a3 3 0 00-5.356-1.857M17 20H7m10 0v-2c0-.656-.126-1.283-.356-1.857M7 20H2v-2a3 3 0 015.356-1.857M7 20v-2c0-.656.126-1.283.356-1.857m0 0a5.002 5.002 0 019.288 0M15 7a3 3 0 11-6 0 3 3 0 016 0z" />
    </svg>
  );
}

function KeyIcon() {
  return (
    <svg className="w-6 h-6" fill="none" viewBox="0 0 24 24" stroke="currentColor">
      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2}
        d="M15 7a2 2 0 012 2m4 0a6 6 0 01-7.743 5.743L11 17H9v2H7v2H4a1 1 0 01-1-1v-2.586a1 1 0 01.293-.707l5.964-5.964A6 6 0 1121 9z" />
    </svg>
  );
}
