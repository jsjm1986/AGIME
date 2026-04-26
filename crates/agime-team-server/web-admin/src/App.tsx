import React, { Suspense } from "react";
import {
  BrowserRouter,
  Routes,
  Route,
  Navigate,
  useLocation,
  useParams,
} from "react-router-dom";
import { useTranslation } from "react-i18next";
import { AuthProvider, useAuth } from "./contexts/AuthContext";
import { BrandProvider } from "./contexts/BrandContext";
import { MobileInteractionModeProvider } from "./contexts/MobileInteractionModeContext";
import { ToastProvider } from "./contexts/ToastContext";
import { RegisterPage } from "./pages/RegisterPage";
import { LoginPage } from "./pages/LoginPage";
import { SystemAdminLoginPage } from "./pages/SystemAdminLoginPage";
import { JoinTeamPage } from "./pages/JoinTeamPage";
import { JoinEntryPage } from "./pages/JoinEntryPage";
import { DashboardPage } from "./pages/DashboardPage";
import { ApiKeysPage } from "./pages/ApiKeysPage";
import { TeamsPage } from "./pages/TeamsPage";
import { SettingsPage } from "./pages/SettingsPage";
import { SystemAdminPage } from "./pages/SystemAdminPage";
import {
  buildRedirectQuery,
  resolveSafeRedirectPath,
} from "./utils/navigation";

const TeamDetailPage = React.lazy(() =>
  import("./pages/TeamDetailPage").then((module) => ({
    default: module.TeamDetailPage,
  })),
);
const AvatarAgentManagerPage = React.lazy(
  () => import("./pages/AvatarAgentManagerPage"),
);
const DigitalAvatarTimelinePage = React.lazy(
  () => import("./pages/DigitalAvatarTimelinePage"),
);
const DigitalAvatarOverviewPage = React.lazy(
  () => import("./pages/DigitalAvatarOverviewPage"),
);
const DigitalAvatarPolicyCenterPage = React.lazy(
  () => import("./pages/DigitalAvatarPolicyCenterPage"),
);
const DigitalAvatarAuditCenterPage = React.lazy(
  () => import("./pages/DigitalAvatarAuditCenterPage"),
);
const SkillRegistryPage = React.lazy(
  () => import("./pages/SkillRegistryPage"),
);
const McpRegistryPage = React.lazy(
  () => import("./pages/McpRegistryPage"),
);
const ChatWorkspacePreviewPage = React.lazy(
  () => import("./pages/ChatWorkspacePreviewPage").then((module) => ({
    default: module.ChatWorkspacePreviewPage,
  })),
);
const PublicChatWorkspacePreviewPage = React.lazy(
  () => import("./pages/PublicChatWorkspacePreviewPage").then((module) => ({
    default: module.PublicChatWorkspacePreviewPage,
  })),
);
const CommandPalette = React.lazy(() =>
  import("./components/ui/command-palette").then((module) => ({
    default: module.CommandPalette,
  })),
);

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
    console.error("[ErrorBoundary]", error, info.componentStack);
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
        {t("errorBoundary.title", "Something went wrong")}
      </h1>
      <p className="text-muted-foreground">
        {t(
          "errorBoundary.description",
          "An unexpected error occurred. Please try reloading the page.",
        )}
      </p>
      <button
        onClick={() => {
          onReset();
          window.location.reload();
        }}
        className="px-4 py-2 rounded bg-primary text-primary-foreground hover:bg-primary/90"
      >
        {t("errorBoundary.reload", "Reload")}
      </button>
    </div>
  );
}

function ProtectedRoute({ children }: { children: React.ReactNode }) {
  const { user, loading, authMode } = useAuth();

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

  if (authMode === "system-admin") {
    return <Navigate to="/system-admin" replace />;
  }

  return <>{children}</>;
}

function RouteLoadingFallback() {
  return (
    <div className="min-h-screen flex items-center justify-center">
      <p>Loading...</p>
    </div>
  );
}

function PublicOnlyRoute({ children }: { children: React.ReactNode }) {
  const { user, loading, authMode } = useAuth();
  const location = useLocation();

  if (loading) {
    return (
      <div className="min-h-screen flex items-center justify-center">
        <p>Loading...</p>
      </div>
    );
  }

  if (user) {
    if (authMode === "system-admin") {
      return <Navigate to="/system-admin" replace />;
    }
    const params = new URLSearchParams(location.search);
    return (
      <Navigate to={resolveSafeRedirectPath(params.get("redirect"))} replace />
    );
  }

  return <>{children}</>;
}

function AdminRoute({ children }: { children: React.ReactNode }) {
  const { user, loading, isSystemAdmin } = useAuth();
  const location = useLocation();

  if (loading) {
    return (
      <div className="min-h-screen flex items-center justify-center">
        <p>Loading...</p>
      </div>
    );
  }

  if (!user) {
    return (
      <Navigate
        to={`/system-admin/login${buildRedirectQuery(`${location.pathname}${location.search}` || "/system-admin")}`}
        replace
      />
    );
  }

  if (!isSystemAdmin) {
    return <Navigate to="/dashboard" replace />;
  }

  return <>{children}</>;
}

// Redirect helper for old routes → TeamDetailPage with ?section=
function TeamSectionRedirect({ section }: { section: string }) {
  const { teamId } = useParams<{ teamId: string }>();
  return <Navigate to={`/teams/${teamId}?section=${section}`} replace />;
}

function AppRoutes() {
  const { user, loading, authMode } = useAuth();

  if (loading) {
    return (
      <div className="min-h-screen flex items-center justify-center">
        <p>Loading...</p>
      </div>
    );
  }

  return (
    <Suspense fallback={<RouteLoadingFallback />}>
      <Routes>
        <Route path="/join" element={<JoinEntryPage />} />
        <Route path="/join/:code" element={<JoinTeamPage />} />
        <Route path="/system-admin/login" element={<SystemAdminLoginPage />} />
        <Route
          path="/register"
          element={
            <PublicOnlyRoute>
              <RegisterPage />
            </PublicOnlyRoute>
          }
        />
        <Route
          path="/login"
          element={
            <PublicOnlyRoute>
              <LoginPage />
            </PublicOnlyRoute>
          }
        />
        <Route
          path="/dashboard"
          element={
            <ProtectedRoute>
              <DashboardPage />
            </ProtectedRoute>
          }
        />
        <Route
          path="/api-keys"
          element={
            <ProtectedRoute>
              <ApiKeysPage />
            </ProtectedRoute>
          }
        />
        <Route
          path="/teams"
          element={
            <ProtectedRoute>
              <TeamsPage />
            </ProtectedRoute>
          }
        />
        <Route
          path="/teams/:teamId"
          element={
            <ProtectedRoute>
              <TeamDetailPage />
            </ProtectedRoute>
          }
        />
        <Route
          path="/teams/:teamId/agent/avatar-managers/:managerId"
          element={
            <ProtectedRoute>
              <AvatarAgentManagerPage />
            </ProtectedRoute>
          }
        />
        <Route
          path="/teams/:teamId/digital-avatars/:avatarId/timeline"
          element={
            <ProtectedRoute>
              <DigitalAvatarTimelinePage />
            </ProtectedRoute>
          }
        />
        <Route
          path="/teams/:teamId/digital-avatars/overview"
          element={
            <ProtectedRoute>
              <DigitalAvatarOverviewPage />
            </ProtectedRoute>
          }
        />
        <Route
          path="/teams/:teamId/digital-avatars/policies"
          element={
            <ProtectedRoute>
              <DigitalAvatarPolicyCenterPage />
            </ProtectedRoute>
          }
        />
        <Route
          path="/teams/:teamId/digital-avatars/audit"
          element={
            <ProtectedRoute>
              <DigitalAvatarAuditCenterPage />
            </ProtectedRoute>
          }
        />
        <Route
          path="/teams/:teamId/skills/registry"
          element={
            <ProtectedRoute>
              <SkillRegistryPage />
            </ProtectedRoute>
          }
        />
        <Route
          path="/teams/:teamId/mcp/workspace"
          element={
            <ProtectedRoute>
              <McpRegistryPage />
            </ProtectedRoute>
          }
        />
        {/* Old routes → redirect to TeamDetailPage with ?section= */}
        <Route
          path="/teams/:teamId/agent"
          element={
            <ProtectedRoute>
              <TeamSectionRedirect section="agent-manage" />
            </ProtectedRoute>
          }
        />
        <Route
          path="/teams/:teamId/chat"
          element={
            <ProtectedRoute>
              <TeamSectionRedirect section="chat" />
            </ProtectedRoute>
          }
        />
        <Route
          path="/teams/:teamId/chat/:sessionId"
          element={
            <ProtectedRoute>
              <TeamSectionRedirect section="chat" />
            </ProtectedRoute>
          }
        />
        <Route
          path="/chat/workspace-preview"
          element={
            <ProtectedRoute>
              <ChatWorkspacePreviewPage />
            </ProtectedRoute>
          }
        />
        <Route
          path="/chat/public-workspace-preview"
          element={<PublicChatWorkspacePreviewPage />}
        />
        <Route
          path="/settings"
          element={
            <ProtectedRoute>
              <SettingsPage />
            </ProtectedRoute>
          }
        />
        <Route
          path="/system-admin"
          element={
            <AdminRoute>
              <SystemAdminPage />
            </AdminRoute>
          }
        />
        <Route
          path="/registrations"
          element={
            <AdminRoute>
              <Navigate to="/system-admin?tab=registrations" replace />
            </AdminRoute>
          }
        />
        <Route
          path="*"
          element={
            <Navigate
              to={
                user
                  ? authMode === "system-admin"
                    ? "/system-admin"
                    : "/dashboard"
                  : "/login"
              }
            />
          }
        />
      </Routes>
    </Suspense>
  );
}

export default function App() {
  return (
    <BrowserRouter basename="/admin">
      <BrandProvider>
        <AuthProvider>
          <MobileInteractionModeProvider>
            <ToastProvider>
              <ErrorBoundary>
                <Suspense fallback={null}>
                  <CommandPalette />
                </Suspense>
                <AppRoutes />
              </ErrorBoundary>
            </ToastProvider>
          </MobileInteractionModeProvider>
        </AuthProvider>
      </BrandProvider>
    </BrowserRouter>
  );
}
