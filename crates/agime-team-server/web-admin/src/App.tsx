import React from 'react';
import { BrowserRouter, Routes, Route, Navigate, useParams } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import { AuthProvider, useAuth } from './contexts/AuthContext';
import { ToastProvider } from './contexts/ToastContext';
import { RegisterPage } from './pages/RegisterPage';
import { LoginPage } from './pages/LoginPage';
import { DashboardPage } from './pages/DashboardPage';
import { ApiKeysPage } from './pages/ApiKeysPage';
import { TeamsPage } from './pages/TeamsPage';
import { TeamDetailPage } from './pages/TeamDetailPage';
import { SettingsPage } from './pages/SettingsPage';
import { RegistrationsPage } from './pages/RegistrationsPage';
import MissionDetailPage from './pages/MissionDetailPage';
import { CommandPalette } from './components/ui/command-palette';

interface ErrorBoundaryState {
  hasError: boolean;
}

class ErrorBoundary extends React.Component<
  { children: React.ReactNode },
  ErrorBoundaryState
> {
  state: ErrorBoundaryState = { hasError: false };

  static getDerivedStateFromError(): ErrorBoundaryState {
    return { hasError: true };
  }

  componentDidCatch(error: Error, info: React.ErrorInfo): void {
    console.error('[ErrorBoundary]', error, info.componentStack);
  }

  render() {
    if (!this.state.hasError) {
      return this.props.children;
    }

    return <ErrorFallback onReset={() => this.setState({ hasError: false })} />;
  }
}

function ErrorFallback({ onReset }: { onReset: () => void }) {
  const { t } = useTranslation();

  return (
    <div className="min-h-screen flex flex-col items-center justify-center gap-4 p-4 text-center">
      <h1 className="text-xl font-semibold">
        {t('errorBoundary.title', 'Something went wrong')}
      </h1>
      <p className="text-muted-foreground">
        {t('errorBoundary.description', 'An unexpected error occurred. Please try reloading the page.')}
      </p>
      <button
        onClick={() => {
          onReset();
          window.location.reload();
        }}
        className="px-4 py-2 rounded bg-primary text-primary-foreground hover:bg-primary/90"
      >
        {t('errorBoundary.reload', 'Reload')}
      </button>
    </div>
  );
}

function ProtectedRoute({ children }: { children: React.ReactNode }) {
  const { user, loading } = useAuth();

  if (loading) {
    return (
      <div className="min-h-screen flex items-center justify-center">
        <p>Loading...</p>
      </div>
    );
  }

  if (!user) {
    return <Navigate to="/login" replace />;
  }

  return <>{children}</>;
}

// Redirect helper for old routes → TeamDetailPage with ?section=
function TeamSectionRedirect({ section }: { section: string }) {
  const { teamId } = useParams<{ teamId: string }>();
  return <Navigate to={`/teams/${teamId}?section=${section}`} replace />;
}

function AppRoutes() {
  const { user, loading } = useAuth();

  if (loading) {
    return (
      <div className="min-h-screen flex items-center justify-center">
        <p>Loading...</p>
      </div>
    );
  }

  return (
    <Routes>
      <Route path="/register" element={<RegisterPage />} />
      <Route path="/login" element={user ? <Navigate to="/dashboard" /> : <LoginPage />} />
      <Route path="/dashboard" element={<ProtectedRoute><DashboardPage /></ProtectedRoute>} />
      <Route path="/api-keys" element={<ProtectedRoute><ApiKeysPage /></ProtectedRoute>} />
      <Route path="/teams" element={<ProtectedRoute><TeamsPage /></ProtectedRoute>} />
      <Route path="/teams/:teamId" element={<ProtectedRoute><TeamDetailPage /></ProtectedRoute>} />
      {/* Old routes → redirect to TeamDetailPage with ?section= */}
      <Route path="/teams/:teamId/agent" element={<ProtectedRoute><TeamSectionRedirect section="agent-manage" /></ProtectedRoute>} />
      <Route path="/teams/:teamId/chat" element={<ProtectedRoute><TeamSectionRedirect section="chat" /></ProtectedRoute>} />
      <Route path="/teams/:teamId/chat/:sessionId" element={<ProtectedRoute><TeamSectionRedirect section="chat" /></ProtectedRoute>} />
      <Route path="/teams/:teamId/missions" element={<ProtectedRoute><TeamSectionRedirect section="missions" /></ProtectedRoute>} />
      {/* Keep MissionDetailPage for deep links */}
      <Route path="/teams/:teamId/missions/:missionId" element={<ProtectedRoute><MissionDetailPage /></ProtectedRoute>} />
      <Route path="/settings" element={<ProtectedRoute><SettingsPage /></ProtectedRoute>} />
      <Route path="/registrations" element={<ProtectedRoute><RegistrationsPage /></ProtectedRoute>} />
      <Route path="*" element={<Navigate to={user ? "/dashboard" : "/login"} />} />
    </Routes>
  );
}

export default function App() {
  return (
    <BrowserRouter basename="/admin">
      <AuthProvider>
        <ToastProvider>
          <ErrorBoundary>
            <CommandPalette />
            <AppRoutes />
          </ErrorBoundary>
        </ToastProvider>
      </AuthProvider>
    </BrowserRouter>
  );
}
