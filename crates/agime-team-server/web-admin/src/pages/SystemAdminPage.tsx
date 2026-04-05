import { useEffect, useMemo, useState, type ReactNode } from "react";
import { useLocation, useNavigate } from "react-router-dom";
import {
  Building2,
  ClipboardList,
  Copy,
  RefreshCw,
  ShieldCheck,
  Users,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import {
  apiClient,
  type AdminRegistrationRequest,
  type AdminUser,
  type SystemAdminOverview,
  type SystemAdminTeam,
} from "../api/client";
import { AppShell } from "../components/layout/AppShell";
import { PageHeader } from "../components/layout/PageHeader";
import { useToast } from "../contexts/ToastContext";
import { copyText } from "../utils/clipboard";
import { formatDateTime } from "../utils/format";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "../components/ui/card";
import {
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
} from "../components/ui/tabs";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "../components/ui/table";
import { Button } from "../components/ui/button";
import { SearchInput } from "../components/ui/search-input";
import { Badge } from "../components/ui/badge";
import { StatusBadge } from "../components/ui/status-badge";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "../components/ui/dialog";
import { Textarea } from "../components/ui/textarea";

type AdminTab = "overview" | "teams" | "users" | "registrations";
interface ApprovalResult {
  email: string;
  apiKey: string;
}

function getActiveTab(search: string): AdminTab {
  const tab = new URLSearchParams(search).get("tab");
  return tab === "teams" || tab === "users" || tab === "registrations"
    ? tab
    : "overview";
}

function SummaryCard({
  icon,
  label,
  value,
  hint,
}: {
  icon: ReactNode;
  label: string;
  value: string;
  hint: string;
}) {
  return (
    <Card className="border-[hsl(var(--border))/0.44] bg-[linear-gradient(180deg,hsl(var(--card))/0.98,hsl(var(--card))/0.78)]">
      <CardContent className="p-5">
        <div className="flex items-start justify-between gap-4">
          <div>
            <p className="text-[11px] font-semibold uppercase tracking-[0.14em] text-[hsl(var(--muted-foreground))/0.92]">
              {label}
            </p>
            <p className="mt-3 font-display text-[34px] font-semibold tracking-[-0.06em] text-[hsl(var(--foreground))]">
              {value}
            </p>
          </div>
          <div className="flex h-11 w-11 items-center justify-center rounded-[16px] border border-[hsl(var(--border))/0.5] bg-[hsl(var(--background))/0.86] text-[hsl(var(--foreground))]">
            {icon}
          </div>
        </div>
        <p className="mt-3 text-sm leading-6 text-[hsl(var(--muted-foreground))/0.92]">
          {hint}
        </p>
      </CardContent>
    </Card>
  );
}

export function SystemAdminPage() {
  const { t } = useTranslation();
  const { addToast } = useToast();
  const location = useLocation();
  const navigate = useNavigate();
  const activeTab = getActiveTab(location.search);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState("");
  const [overview, setOverview] = useState<SystemAdminOverview | null>(null);
  const [teams, setTeams] = useState<SystemAdminTeam[]>([]);
  const [users, setUsers] = useState<AdminUser[]>([]);
  const [pendingRequests, setPendingRequests] = useState<
    AdminRegistrationRequest[]
  >([]);
  const [historyRequests, setHistoryRequests] = useState<
    AdminRegistrationRequest[]
  >([]);
  const [teamSearch, setTeamSearch] = useState("");
  const [userSearch, setUserSearch] = useState("");
  const [rejectDialogRequest, setRejectDialogRequest] =
    useState<AdminRegistrationRequest | null>(null);
  const [rejectReason, setRejectReason] = useState("");
  const [approvalResult, setApprovalResult] = useState<ApprovalResult | null>(
    null,
  );
  const [actionKey, setActionKey] = useState("");

  const loadAll = async (silent = false) => {
    silent ? setRefreshing(true) : setLoading(true);
    try {
      setError("");
      const [overviewRes, teamsRes, usersRes, pendingRes, historyRes] =
        await Promise.all([
          apiClient.getSystemAdminOverview(),
          apiClient.getSystemAdminTeams(),
          apiClient.getAdminUsers(),
          apiClient.getRegistrations(),
          apiClient.getRegistrationHistory(),
        ]);
      setOverview(overviewRes.overview);
      setTeams(teamsRes.teams);
      setUsers(usersRes.users);
      setPendingRequests(pendingRes.requests);
      setHistoryRequests(historyRes.requests);
    } catch (err) {
      const message = err instanceof Error ? err.message : t("common.error");
      setError(message);
      addToast("error", message);
    } finally {
      setLoading(false);
      setRefreshing(false);
    }
  };

  useEffect(() => {
    loadAll();
  }, []);

  const filteredTeams = useMemo(() => {
    const q = teamSearch.trim().toLowerCase();
    return q
      ? teams.filter((team) =>
          [team.name, team.id].join(" ").toLowerCase().includes(q),
        )
      : teams;
  }, [teamSearch, teams]);

  const filteredUsers = useMemo(() => {
    const q = userSearch.trim().toLowerCase();
    return q
      ? users.filter((entry) =>
          [entry.display_name, entry.email, entry.id]
            .join(" ")
            .toLowerCase()
            .includes(q),
        )
      : users;
  }, [userSearch, users]);

  const handleApprove = async (request: AdminRegistrationRequest) => {
    setActionKey(`approve:${request.request_id}`);
    try {
      const response = await apiClient.approveRegistration(request.request_id);
      setApprovalResult({ email: request.email, apiKey: response.api_key });
      addToast("success", t("registrations.approved"));
      await loadAll(true);
    } catch (err) {
      addToast("error", err instanceof Error ? err.message : t("common.error"));
    } finally {
      setActionKey("");
    }
  };

  const handleReject = async () => {
    if (!rejectDialogRequest) return;
    setActionKey(`reject:${rejectDialogRequest.request_id}`);
    try {
      await apiClient.rejectRegistration(
        rejectDialogRequest.request_id,
        rejectReason.trim() || undefined,
      );
      addToast("success", t("registrations.rejected"));
      setRejectDialogRequest(null);
      setRejectReason("");
      await loadAll(true);
    } catch (err) {
      addToast("error", err instanceof Error ? err.message : t("common.error"));
    } finally {
      setActionKey("");
    }
  };

  const handleCopy = async (value: string) => {
    if (await copyText(value)) {
      addToast("success", t("systemAdmin.registrations.apiKeyCopied"));
      return;
    }
    addToast("error", t("common.error"));
  };

  const newestTeams = teams.slice(0, 5);
  const newestUsers = users.slice(0, 5);

  return (
    <AppShell>
      <PageHeader
        title={t("systemAdmin.title")}
        description={t("systemAdmin.description")}
        actions={
          <Button
            variant="outline"
            size="sm"
            onClick={() => loadAll(true)}
            disabled={refreshing}
          >
            <RefreshCw
              className={`h-3.5 w-3.5 ${refreshing ? "animate-spin" : ""}`}
            />
            {refreshing ? t("common.loading") : t("common.reload")}
          </Button>
        }
      />
      <div className="space-y-6">
        <Card className="overflow-hidden border-[hsl(var(--border))/0.48] bg-[radial-gradient(circle_at_top_left,hsl(var(--primary))/0.14,transparent_32%),linear-gradient(180deg,hsl(var(--card))/0.99,hsl(var(--card))/0.8)]">
          <CardContent className="grid gap-6 px-6 py-6 lg:grid-cols-[minmax(0,1.1fr)_minmax(0,0.9fr)]">
            <div className="space-y-3">
              <div className="inline-flex items-center gap-2 rounded-full border border-[hsl(var(--border))/0.46] bg-[hsl(var(--background))/0.64] px-3 py-1 text-[11px] font-semibold uppercase tracking-[0.14em] text-[hsl(var(--muted-foreground))/0.94]">
                <ShieldCheck className="h-3.5 w-3.5" />
                {t("systemAdmin.hero.eyebrow")}
              </div>
              <h2 className="max-w-2xl font-display text-[32px] font-semibold tracking-[-0.05em] text-[hsl(var(--foreground))] md:text-[38px]">
                {t("systemAdmin.hero.title")}
              </h2>
              <p className="max-w-2xl text-sm leading-7 text-[hsl(var(--muted-foreground))/0.94]">
                {t("systemAdmin.hero.description")}
              </p>
            </div>
            <div className="rounded-[24px] border border-[hsl(var(--border))/0.46] bg-[hsl(var(--background))/0.66] p-5">
              <p className="text-[11px] font-semibold uppercase tracking-[0.14em] text-[hsl(var(--muted-foreground))/0.92]">
                {t("systemAdmin.overview.title")}
              </p>
              <div className="mt-4 grid gap-3 sm:grid-cols-2">
                <div className="rounded-[18px] border border-[hsl(var(--border))/0.36] bg-[hsl(var(--card))/0.9] px-4 py-3">
                  <p className="text-xs text-[hsl(var(--muted-foreground))]">
                    {t("systemAdmin.summary.totalUsers")}
                  </p>
                  <p className="mt-1 text-2xl font-semibold text-[hsl(var(--foreground))]">
                    {overview?.total_users ?? "—"}
                  </p>
                </div>
                <div className="rounded-[18px] border border-[hsl(var(--border))/0.36] bg-[hsl(var(--card))/0.9] px-4 py-3">
                  <p className="text-xs text-[hsl(var(--muted-foreground))]">
                    {t("systemAdmin.summary.totalTeams")}
                  </p>
                  <p className="mt-1 text-2xl font-semibold text-[hsl(var(--foreground))]">
                    {overview?.total_teams ?? "—"}
                  </p>
                </div>
                <div className="rounded-[18px] border border-[hsl(var(--border))/0.36] bg-[hsl(var(--card))/0.9] px-4 py-3">
                  <p className="text-xs text-[hsl(var(--muted-foreground))]">
                    {t("systemAdmin.summary.admins")}
                  </p>
                  <p className="mt-1 text-2xl font-semibold text-[hsl(var(--foreground))]">
                    {overview?.total_system_admins ?? "—"}
                  </p>
                </div>
                <div className="rounded-[18px] border border-[hsl(var(--border))/0.36] bg-[hsl(var(--card))/0.9] px-4 py-3">
                  <p className="text-xs text-[hsl(var(--muted-foreground))]">
                    {t("systemAdmin.summary.pending")}
                  </p>
                  <p className="mt-1 text-2xl font-semibold text-[hsl(var(--foreground))]">
                    {overview?.pending_registrations ?? "—"}
                  </p>
                </div>
              </div>
            </div>
          </CardContent>
        </Card>

        {error && (
          <Card className="border-[hsl(var(--destructive))/0.26] bg-[hsl(var(--destructive))/0.06]">
            <CardContent className="p-4 text-sm text-[hsl(var(--destructive))]">
              {error}
            </CardContent>
          </Card>
        )}

        <Tabs
          value={activeTab}
          onValueChange={(value) =>
            navigate(
              value === "overview"
                ? "/system-admin"
                : `/system-admin?tab=${value}`,
              { replace: true },
            )
          }
        >
          <TabsList className="w-full justify-start gap-2 overflow-x-auto rounded-[18px] border border-[hsl(var(--border))/0.46] bg-[hsl(var(--card))/0.84] p-1">
            <TabsTrigger value="overview">
              {t("systemAdmin.tabs.overview")}
            </TabsTrigger>
            <TabsTrigger value="teams">
              {t("systemAdmin.tabs.teams")}
            </TabsTrigger>
            <TabsTrigger value="users">
              {t("systemAdmin.tabs.users")}
            </TabsTrigger>
            <TabsTrigger value="registrations">
              {t("systemAdmin.tabs.registrations")}
            </TabsTrigger>
          </TabsList>

          <TabsContent value="overview" className="space-y-4">
            <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
              <SummaryCard
                icon={<Users className="h-5 w-5" />}
                label={t("systemAdmin.summary.totalUsers")}
                value={overview ? String(overview.total_users) : "—"}
                hint={t("systemAdmin.summary.totalUsersHint")}
              />
              <SummaryCard
                icon={<Building2 className="h-5 w-5" />}
                label={t("systemAdmin.summary.totalTeams")}
                value={overview ? String(overview.total_teams) : "—"}
                hint={t("systemAdmin.summary.totalTeamsHint")}
              />
              <SummaryCard
                icon={<ShieldCheck className="h-5 w-5" />}
                label={t("systemAdmin.summary.admins")}
                value={overview ? String(overview.total_system_admins) : "—"}
                hint={t("systemAdmin.summary.adminsHint")}
              />
              <SummaryCard
                icon={<ClipboardList className="h-5 w-5" />}
                label={t("systemAdmin.summary.pending")}
                value={overview ? String(overview.pending_registrations) : "—"}
                hint={t("systemAdmin.summary.pendingHint")}
              />
            </div>
            <div className="grid gap-4 xl:grid-cols-2">
              <Card className="border-[hsl(var(--border))/0.44]">
                <CardHeader>
                  <CardTitle>{t("systemAdmin.overview.recentTeams")}</CardTitle>
                  <CardDescription>
                    {t("systemAdmin.overview.recentTeamsDescription")}
                  </CardDescription>
                </CardHeader>
                <CardContent className="space-y-3">
                  {newestTeams.length === 0 ? (
                    <div className="rounded-[18px] border border-dashed border-[hsl(var(--border))/0.42] bg-[hsl(var(--background))/0.55] px-4 py-8 text-center text-sm text-[hsl(var(--muted-foreground))]">
                      {t("systemAdmin.overview.empty")}
                    </div>
                  ) : (
                    newestTeams.map((team) => (
                      <div
                        key={team.id}
                        className="flex items-start justify-between gap-4 rounded-[18px] border border-[hsl(var(--border))/0.36] bg-[hsl(var(--background))/0.55] px-4 py-3"
                      >
                        <div className="min-w-0">
                          <p className="truncate text-sm font-medium text-[hsl(var(--foreground))]">
                            {team.name}
                          </p>
                          <p className="truncate text-xs text-[hsl(var(--muted-foreground))]">
                            {team.id}
                          </p>
                        </div>
                        <p className="shrink-0 text-xs font-medium text-[hsl(var(--muted-foreground))/0.88]">
                          {t("systemAdmin.teams.memberCount", {
                            count: team.members_count,
                          })}
                        </p>
                      </div>
                    ))
                  )}
                </CardContent>
              </Card>
              <Card className="border-[hsl(var(--border))/0.44]">
                <CardHeader>
                  <CardTitle>{t("systemAdmin.overview.recentUsers")}</CardTitle>
                  <CardDescription>
                    {t("systemAdmin.overview.recentUsersDescription")}
                  </CardDescription>
                </CardHeader>
                <CardContent className="space-y-3">
                  {newestUsers.length === 0 ? (
                    <div className="rounded-[18px] border border-dashed border-[hsl(var(--border))/0.42] bg-[hsl(var(--background))/0.55] px-4 py-8 text-center text-sm text-[hsl(var(--muted-foreground))]">
                      {t("systemAdmin.overview.empty")}
                    </div>
                  ) : (
                    newestUsers.map((entry) => (
                      <div
                        key={entry.id}
                        className="flex items-start justify-between gap-4 rounded-[18px] border border-[hsl(var(--border))/0.36] bg-[hsl(var(--background))/0.55] px-4 py-3"
                      >
                        <div className="min-w-0">
                          <p className="truncate text-sm font-medium text-[hsl(var(--foreground))]">
                            {entry.display_name}
                          </p>
                          <p className="truncate text-xs text-[hsl(var(--muted-foreground))]">
                            {entry.email}
                          </p>
                        </div>
                        <p className="shrink-0 text-xs font-medium text-[hsl(var(--muted-foreground))/0.88]">
                          {formatDateTime(entry.created_at)}
                        </p>
                      </div>
                    ))
                  )}
                </CardContent>
              </Card>
            </div>
          </TabsContent>

          <TabsContent value="teams">
            <Card className="border-[hsl(var(--border))/0.44]">
              <CardHeader className="gap-3 lg:flex-row lg:items-end lg:justify-between">
                <div>
                  <CardTitle>{t("systemAdmin.teams.title")}</CardTitle>
                  <CardDescription>
                    {t("systemAdmin.teams.description")}
                  </CardDescription>
                </div>
                <div className="w-full max-w-[320px]">
                  <SearchInput
                    value={teamSearch}
                    onChange={(event) => setTeamSearch(event.target.value)}
                    onClear={() => setTeamSearch("")}
                    placeholder={t("systemAdmin.teams.searchPlaceholder")}
                  />
                </div>
              </CardHeader>
              <CardContent className="overflow-x-auto">
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead>
                        {t("systemAdmin.teams.columns.team")}
                      </TableHead>
                      <TableHead>
                        {t("systemAdmin.teams.columns.members")}
                      </TableHead>
                      <TableHead>
                        {t("systemAdmin.teams.columns.created")}
                      </TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {loading ? (
                      <TableRow>
                        <TableCell
                          colSpan={3}
                          className="py-8 text-center text-[hsl(var(--muted-foreground))]"
                        >
                          {t("common.loading")}
                        </TableCell>
                      </TableRow>
                    ) : filteredTeams.length === 0 ? (
                      <TableRow>
                        <TableCell
                          colSpan={3}
                          className="py-8 text-center text-[hsl(var(--muted-foreground))]"
                        >
                          {t("systemAdmin.teams.empty")}
                        </TableCell>
                      </TableRow>
                    ) : (
                      filteredTeams.map((team) => (
                        <TableRow key={team.id}>
                          <TableCell className="min-w-[260px]">
                            <div className="space-y-1">
                              <p className="font-medium text-[hsl(var(--foreground))]">
                                {team.name}
                              </p>
                              <p className="text-xs text-[hsl(var(--muted-foreground))]">
                                {team.id}
                              </p>
                            </div>
                          </TableCell>
                          <TableCell>
                            <Badge variant="outline">
                              {t("systemAdmin.teams.memberCount", {
                                count: team.members_count,
                              })}
                            </Badge>
                          </TableCell>
                          <TableCell>
                            {formatDateTime(team.created_at)}
                          </TableCell>
                        </TableRow>
                      ))
                    )}
                  </TableBody>
                </Table>
              </CardContent>
            </Card>
          </TabsContent>

          <TabsContent value="users">
            <Card className="border-[hsl(var(--border))/0.44]">
              <CardHeader className="gap-3 lg:flex-row lg:items-end lg:justify-between">
                <div>
                  <CardTitle>{t("systemAdmin.users.title")}</CardTitle>
                  <CardDescription>
                    {t("systemAdmin.users.description")}
                  </CardDescription>
                </div>
                <div className="w-full max-w-[320px]">
                  <SearchInput
                    value={userSearch}
                    onChange={(event) => setUserSearch(event.target.value)}
                    onClear={() => setUserSearch("")}
                    placeholder={t("systemAdmin.users.searchPlaceholder")}
                  />
                </div>
              </CardHeader>
              <CardContent className="overflow-x-auto">
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead>
                        {t("systemAdmin.users.columns.user")}
                      </TableHead>
                      <TableHead>
                        {t("systemAdmin.users.columns.role")}
                      </TableHead>
                      <TableHead>
                        {t("systemAdmin.users.columns.status")}
                      </TableHead>
                      <TableHead>
                        {t("systemAdmin.users.columns.created")}
                      </TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {loading ? (
                      <TableRow>
                        <TableCell
                          colSpan={4}
                          className="py-8 text-center text-[hsl(var(--muted-foreground))]"
                        >
                          {t("common.loading")}
                        </TableCell>
                      </TableRow>
                    ) : filteredUsers.length === 0 ? (
                      <TableRow>
                        <TableCell
                          colSpan={4}
                          className="py-8 text-center text-[hsl(var(--muted-foreground))]"
                        >
                          {t("systemAdmin.users.empty")}
                        </TableCell>
                      </TableRow>
                    ) : (
                      filteredUsers.map((entry) => (
                        <TableRow key={entry.id}>
                          <TableCell className="min-w-[260px]">
                            <div className="space-y-1">
                              <p className="font-medium text-[hsl(var(--foreground))]">
                                {entry.display_name}
                              </p>
                              <p className="text-xs text-[hsl(var(--muted-foreground))]">
                                {entry.email}
                              </p>
                            </div>
                          </TableCell>
                          <TableCell>
                            <Badge
                              variant={
                                entry.role === "admin" ? "secondary" : "outline"
                              }
                            >
                              {t(`systemAdmin.users.roles.${entry.role}`)}
                            </Badge>
                          </TableCell>
                          <TableCell>
                            <StatusBadge
                              status={entry.is_active ? "success" : "neutral"}
                            >
                              {entry.is_active
                                ? t("systemAdmin.users.status.active")
                                : t("systemAdmin.users.status.inactive")}
                            </StatusBadge>
                          </TableCell>
                          <TableCell>
                            {formatDateTime(entry.created_at)}
                          </TableCell>
                        </TableRow>
                      ))
                    )}
                  </TableBody>
                </Table>
              </CardContent>
            </Card>
          </TabsContent>

          <TabsContent value="registrations" className="space-y-4">
            <Card className="border-[hsl(var(--border))/0.44]">
              <CardHeader>
                <CardTitle>
                  {t("systemAdmin.registrations.pendingTitle")}
                </CardTitle>
                <CardDescription>
                  {t("systemAdmin.registrations.pendingDescription")}
                </CardDescription>
              </CardHeader>
              <CardContent className="overflow-x-auto">
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead>
                        {t("systemAdmin.registrations.columns.applicant")}
                      </TableHead>
                      <TableHead>
                        {t("systemAdmin.registrations.columns.created")}
                      </TableHead>
                      <TableHead>
                        {t("systemAdmin.registrations.columns.credentials")}
                      </TableHead>
                      <TableHead className="w-[210px]">
                        {t("common.actions")}
                      </TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {loading ? (
                      <TableRow>
                        <TableCell
                          colSpan={4}
                          className="py-8 text-center text-[hsl(var(--muted-foreground))]"
                        >
                          {t("common.loading")}
                        </TableCell>
                      </TableRow>
                    ) : pendingRequests.length === 0 ? (
                      <TableRow>
                        <TableCell
                          colSpan={4}
                          className="py-8 text-center text-[hsl(var(--muted-foreground))]"
                        >
                          {t("registrations.noRequests")}
                        </TableCell>
                      </TableRow>
                    ) : (
                      pendingRequests.map((request) => (
                        <TableRow key={request.request_id}>
                          <TableCell className="min-w-[240px]">
                            <div className="space-y-1">
                              <p className="font-medium">
                                {request.display_name}
                              </p>
                              <p className="text-xs text-[hsl(var(--muted-foreground))]">
                                {request.email}
                              </p>
                            </div>
                          </TableCell>
                          <TableCell>
                            {formatDateTime(request.created_at)}
                          </TableCell>
                          <TableCell>
                            {request.has_password
                              ? t("systemAdmin.registrations.passwordProvided")
                              : t("systemAdmin.registrations.passwordMissing")}
                          </TableCell>
                          <TableCell>
                            <div className="flex flex-wrap gap-2">
                              <Button
                                size="sm"
                                onClick={() => handleApprove(request)}
                                disabled={
                                  actionKey === `approve:${request.request_id}`
                                }
                              >
                                {t("registrations.approve")}
                              </Button>
                              <Button
                                size="sm"
                                variant="outline"
                                onClick={() => {
                                  setRejectDialogRequest(request);
                                  setRejectReason("");
                                }}
                                disabled={
                                  actionKey === `reject:${request.request_id}`
                                }
                              >
                                {t("registrations.reject")}
                              </Button>
                            </div>
                          </TableCell>
                        </TableRow>
                      ))
                    )}
                  </TableBody>
                </Table>
              </CardContent>
            </Card>
            <Card className="border-[hsl(var(--border))/0.44]">
              <CardHeader>
                <CardTitle>
                  {t("systemAdmin.registrations.historyTitle")}
                </CardTitle>
                <CardDescription>
                  {t("systemAdmin.registrations.historyDescription")}
                </CardDescription>
              </CardHeader>
              <CardContent className="overflow-x-auto">
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead>
                        {t("systemAdmin.registrations.columns.applicant")}
                      </TableHead>
                      <TableHead>
                        {t("systemAdmin.registrations.columns.status")}
                      </TableHead>
                      <TableHead>
                        {t("systemAdmin.registrations.columns.reviewedBy")}
                      </TableHead>
                      <TableHead>
                        {t("systemAdmin.registrations.columns.reviewedAt")}
                      </TableHead>
                      <TableHead>
                        {t("systemAdmin.registrations.columns.reason")}
                      </TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {historyRequests.length === 0 ? (
                      <TableRow>
                        <TableCell
                          colSpan={5}
                          className="py-8 text-center text-[hsl(var(--muted-foreground))]"
                        >
                          {t("systemAdmin.registrations.historyEmpty")}
                        </TableCell>
                      </TableRow>
                    ) : (
                      historyRequests.map((request) => (
                        <TableRow key={request.request_id}>
                          <TableCell className="min-w-[240px]">
                            <div className="space-y-1">
                              <p className="font-medium">
                                {request.display_name}
                              </p>
                              <p className="text-xs text-[hsl(var(--muted-foreground))]">
                                {request.email}
                              </p>
                            </div>
                          </TableCell>
                          <TableCell>
                            <StatusBadge
                              status={
                                request.status === "approved"
                                  ? "success"
                                  : "error"
                              }
                            >
                              {t(
                                `systemAdmin.registrations.status.${request.status}`,
                              )}
                            </StatusBadge>
                          </TableCell>
                          <TableCell>{request.reviewed_by || "-"}</TableCell>
                          <TableCell>
                            {request.reviewed_at
                              ? formatDateTime(request.reviewed_at)
                              : "-"}
                          </TableCell>
                          <TableCell className="min-w-[220px] text-sm text-[hsl(var(--muted-foreground))]">
                            {request.reject_reason || "—"}
                          </TableCell>
                        </TableRow>
                      ))
                    )}
                  </TableBody>
                </Table>
              </CardContent>
            </Card>
          </TabsContent>
        </Tabs>
      </div>

      <Dialog
        open={!!rejectDialogRequest}
        onOpenChange={() => setRejectDialogRequest(null)}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>
              {t("systemAdmin.registrations.rejectDialogTitle")}
            </DialogTitle>
            <DialogDescription>
              {t("systemAdmin.registrations.rejectDialogDescription")}
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-3 py-2">
            <div className="rounded-[14px] border border-[hsl(var(--border))/0.4] bg-[hsl(var(--background))/0.52] p-4">
              <p className="font-medium">{rejectDialogRequest?.display_name}</p>
              <p className="mt-1 text-sm text-[hsl(var(--muted-foreground))]">
                {rejectDialogRequest?.email}
              </p>
            </div>
            <Textarea
              value={rejectReason}
              onChange={(event) => setRejectReason(event.target.value)}
              placeholder={t(
                "systemAdmin.registrations.rejectReasonPlaceholder",
              )}
            />
          </div>
          <DialogFooter>
            <Button
              variant="outline"
              onClick={() => setRejectDialogRequest(null)}
            >
              {t("common.cancel")}
            </Button>
            <Button
              variant="destructive"
              onClick={handleReject}
              disabled={actionKey.startsWith("reject:")}
            >
              {actionKey.startsWith("reject:")
                ? t("common.saving")
                : t("registrations.reject")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
      <Dialog
        open={!!approvalResult}
        onOpenChange={() => setApprovalResult(null)}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>
              {t("systemAdmin.registrations.approvalResultTitle")}
            </DialogTitle>
            <DialogDescription>
              {t("systemAdmin.registrations.approvalResultDescription")}
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-4 py-2">
            <div className="rounded-[14px] border border-[hsl(var(--status-success-text))/0.18] bg-status-success-bg p-4">
              <p className="text-sm font-medium text-status-success-text">
                {approvalResult?.email}
              </p>
              <p className="mt-2 break-all rounded-[12px] border border-[hsl(var(--border))/0.32] bg-[hsl(var(--background))/0.9] px-3 py-3 font-mono text-xs text-[hsl(var(--foreground))]">
                {approvalResult?.apiKey}
              </p>
            </div>
            <p className="text-sm text-[hsl(var(--muted-foreground))]">
              {t("systemAdmin.registrations.approvalResultWarning")}
            </p>
          </div>
          <DialogFooter>
            <Button
              variant="outline"
              onClick={() =>
                approvalResult && handleCopy(approvalResult.apiKey)
              }
            >
              <Copy className="h-3.5 w-3.5" />
              {t("systemAdmin.registrations.copyApiKey")}
            </Button>
            <Button onClick={() => setApprovalResult(null)}>
              {t("common.done")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </AppShell>
  );
}
