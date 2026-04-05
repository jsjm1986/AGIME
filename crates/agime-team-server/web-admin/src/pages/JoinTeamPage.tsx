import { useEffect, useMemo, useState } from "react";
import { Link, useNavigate, useParams } from "react-router-dom";
import { useTranslation } from "react-i18next";
import {
  AlertTriangle,
  ArrowRight,
  CheckCircle2,
  Loader2,
  LogIn,
  ShieldCheck,
  UserPlus,
  Users,
} from "lucide-react";
import { LanguageSwitcher } from "../components/LanguageSwitcher";
import { Badge } from "../components/ui/badge";
import { Button } from "../components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "../components/ui/card";
import { apiClient } from "../api/client";
import type {
  AcceptInviteResponse,
  ValidateInviteResponse,
} from "../api/types";
import { useAuth } from "../contexts/AuthContext";
import { useBrand } from "../contexts/BrandContext";
import { formatDateTime } from "../utils/format";
import { buildRedirectQuery } from "../utils/navigation";

export function JoinTeamPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { code = "" } = useParams<{ code: string }>();
  const { user, loading: authLoading } = useAuth();
  const platformUser = user?.auth_mode === "system-admin" ? null : user;
  const { brand } = useBrand();
  const [invite, setInvite] = useState<ValidateInviteResponse | null>(null);
  const [inviteError, setInviteError] = useState("");
  const [loadingInvite, setLoadingInvite] = useState(true);
  const [accepting, setAccepting] = useState(false);
  const [acceptError, setAcceptError] = useState("");
  const [acceptResult, setAcceptResult] = useState<AcceptInviteResponse | null>(
    null,
  );
  const [autoAccepted, setAutoAccepted] = useState(false);

  useEffect(() => {
    let cancelled = false;

    async function loadInvite() {
      if (!code) {
        setInvite(null);
        setInviteError(t("join.invalidFallback"));
        setLoadingInvite(false);
        return;
      }

      setLoadingInvite(true);
      setInvite(null);
      setInviteError("");
      setAcceptError("");
      setAcceptResult(null);

      try {
        const response = await apiClient.validateInvite(code);
        if (cancelled) {
          return;
        }
        setInvite(response);
        if (!response.valid) {
          setInviteError(response.error || t("join.invalidFallback"));
        }
      } catch (err) {
        if (cancelled) {
          return;
        }
        setInvite(null);
        setInviteError(err instanceof Error ? err.message : t("common.error"));
      } finally {
        if (!cancelled) {
          setLoadingInvite(false);
        }
      }
    }

    loadInvite();

    return () => {
      cancelled = true;
    };
  }, [code, t]);

  const redirectQuery = useMemo(
    () => buildRedirectQuery(`/join/${code}`),
    [code],
  );
  const registerQuery = useMemo(() => {
    const params = new URLSearchParams();
    params.set("redirect", `/join/${code}`);
    params.set("invite", code);
    return `?${params.toString()}`;
  }, [code]);

  const teamId = acceptResult?.teamId ?? invite?.teamId ?? null;
  const teamName =
    acceptResult?.teamName ?? invite?.teamName ?? t("join.pendingTeam");
  const inviteeEmailHint = invite?.inviteeEmailHint ?? null;
  const isOpenInvite = invite?.isOpenInvite ?? false;
  const alreadyMember = Boolean(
    acceptResult?.error?.toLowerCase().includes("already a member"),
  );

  const handleAccept = async () => {
    if (!code || !platformUser) {
      return;
    }

    setAccepting(true);
    setAcceptError("");

    try {
      const response = await apiClient.acceptInvite(
        code,
        platformUser.display_name || platformUser.email,
      );
      const isAlreadyMember = Boolean(
        response.error?.toLowerCase().includes("already a member"),
      );

      if (response.success || isAlreadyMember) {
        setAcceptResult(
          isAlreadyMember ? { ...response, success: true } : response,
        );
        return;
      }

      setAcceptError(response.error || t("join.acceptFailed"));
    } catch (err) {
      setAcceptError(
        err instanceof Error ? err.message : t("join.acceptFailed"),
      );
    } finally {
      setAccepting(false);
    }
  };

  useEffect(() => {
    if (
      autoAccepted ||
      loadingInvite ||
      authLoading ||
      !invite?.valid ||
      !platformUser ||
      acceptResult ||
      accepting
    ) {
      return;
    }

    setAutoAccepted(true);
    void handleAccept();
  }, [
    acceptResult,
    accepting,
    authLoading,
    autoAccepted,
    invite,
    loadingInvite,
    platformUser,
  ]);

  const renderActionCard = () => {
    if (loadingInvite) {
      return (
        <Card className="border-[hsl(var(--border))/0.4] bg-[hsl(var(--card))/0.9]">
          <CardHeader>
            <CardTitle>{t("join.loading")}</CardTitle>
            <CardDescription>{t("join.loadingDescription")}</CardDescription>
          </CardHeader>
          <CardContent className="flex items-center gap-3 text-sm text-[hsl(var(--muted-foreground))]">
            <Loader2 className="h-4 w-4 animate-spin" />
            <span>{t("join.sessionChecking")}</span>
          </CardContent>
        </Card>
      );
    }

    if (!invite?.valid) {
      return (
        <Card className="border-[hsl(var(--destructive))/0.25] bg-[hsl(var(--card))/0.92]">
          <CardHeader>
            <div className="flex items-center gap-3">
              <div className="rounded-full bg-[hsl(var(--destructive))]/10 p-2 text-[hsl(var(--destructive))]">
                <AlertTriangle className="h-5 w-5" />
              </div>
              <div>
                <CardTitle>{t("join.invalidTitle")}</CardTitle>
                <CardDescription>
                  {inviteError || t("join.invalidFallback")}
                </CardDescription>
              </div>
            </div>
          </CardHeader>
          <CardFooter className="flex flex-col gap-3 sm:flex-row">
            <Button variant="outline" className="w-full sm:flex-1" asChild>
              <Link to="/login">{t("join.backToLogin")}</Link>
            </Button>
            <Button className="w-full sm:flex-1" asChild>
              <Link to="/teams">{t("join.viewTeams")}</Link>
            </Button>
          </CardFooter>
        </Card>
      );
    }

    if (acceptResult) {
      return (
        <Card className="ui-section-panel border-[hsl(var(--status-success-text))/0.18] bg-[hsl(var(--status-success-bg))/0.46]">
          <CardHeader>
            <div className="flex items-center gap-3">
              <div className="rounded-full bg-[hsl(var(--status-success-bg))] p-2 text-[hsl(var(--status-success-text))]">
                <CheckCircle2 className="h-5 w-5" />
              </div>
              <div>
                <CardTitle>
                  {alreadyMember
                    ? t("join.alreadyMemberTitle")
                    : t("join.successTitle")}
                </CardTitle>
                <CardDescription>
                  {alreadyMember
                    ? t("join.alreadyMemberDescription", { team: teamName })
                    : t("join.successDescription", { team: teamName })}
                </CardDescription>
              </div>
            </div>
          </CardHeader>
          <CardFooter className="flex flex-col gap-3 sm:flex-row">
            <Button variant="outline" className="w-full sm:flex-1" asChild>
              <Link to="/teams">{t("join.viewTeams")}</Link>
            </Button>
            {teamId && (
              <Button className="w-full sm:flex-1" asChild>
                <Link to={`/teams/${teamId}`}>
                  {t("join.goToTeam")}
                  <ArrowRight className="h-4 w-4" />
                </Link>
              </Button>
            )}
          </CardFooter>
        </Card>
      );
    }

    if (authLoading) {
      return (
        <Card className="border-[hsl(var(--border))/0.4] bg-[hsl(var(--card))/0.9]">
          <CardHeader>
            <CardTitle>{t("join.loading")}</CardTitle>
            <CardDescription>{t("join.loadingDescription")}</CardDescription>
          </CardHeader>
          <CardContent className="flex items-center gap-3 text-sm text-[hsl(var(--muted-foreground))]">
            <Loader2 className="h-4 w-4 animate-spin" />
            <span>{t("join.sessionChecking")}</span>
          </CardContent>
        </Card>
      );
    }

    if (!platformUser) {
      return (
        <Card className="border-[hsl(var(--border))/0.4] bg-[hsl(var(--card))/0.92]">
          <CardHeader>
            <CardTitle>{t("join.loginRequiredTitle")}</CardTitle>
            <CardDescription>
              {t("join.loginRequiredDescription")}
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="rounded-2xl border border-[hsl(var(--border))/0.36] bg-[hsl(var(--muted))]/0.35 p-4">
              <div className="flex items-center gap-2 text-sm font-medium">
                <ShieldCheck className="h-4 w-4 text-[hsl(var(--primary))]" />
                {t("join.acceptHint")}
              </div>
              <p className="mt-2 text-sm text-[hsl(var(--muted-foreground))]">
                {isOpenInvite ? t("join.openInviteDescription") : t("join.description")}
              </p>
              {inviteeEmailHint && (
                <p className="mt-3 text-sm text-[hsl(var(--foreground))]">
                  {t("join.inviteeEmailLabel")} {inviteeEmailHint}
                </p>
              )}
            </div>
          </CardContent>
          <CardFooter className="flex flex-col gap-3 sm:flex-row">
            <Button variant="outline" className="w-full sm:flex-1" asChild>
              <Link to={`/register${registerQuery}`}>
                <UserPlus className="h-4 w-4" />
                {t("join.goToRegister")}
              </Link>
            </Button>
            <Button className="w-full sm:flex-1" asChild>
              <Link to={`/login${redirectQuery}`}>
                <LogIn className="h-4 w-4" />
                {t("join.goToLogin")}
              </Link>
            </Button>
          </CardFooter>
        </Card>
      );
    }

    return (
        <Card className="border-[hsl(var(--border))/0.4] bg-[hsl(var(--card))/0.92]">
          <CardHeader>
            <CardTitle>{t("join.readyTitle")}</CardTitle>
            <CardDescription>
              {accepting
                ? t("join.autoAccepting", {
                    defaultValue: "检测到当前账号已登录，正在自动加入团队...",
                  })
                : t("join.acceptHint")}
            </CardDescription>
          </CardHeader>
        <CardContent className="space-y-4">
          <div className="rounded-2xl border border-[hsl(var(--border))/0.36] bg-[hsl(var(--muted))]/0.35 p-4">
            <div className="text-xs uppercase tracking-[0.18em] text-[hsl(var(--muted-foreground))]">
              {t("join.currentUserLabel")}
            </div>
            <div className="mt-2 text-base font-semibold">
              {platformUser.display_name || platformUser.email}
            </div>
            <div className="mt-1 text-sm text-[hsl(var(--muted-foreground))]">
              {platformUser.email}
            </div>
          </div>
          {acceptError && (
            <div className="rounded-2xl border border-[hsl(var(--destructive))/0.25] bg-[hsl(var(--destructive))]/5 px-4 py-3 text-sm text-[hsl(var(--destructive))]">
              {acceptError}
            </div>
          )}
        </CardContent>
        <CardFooter className="flex flex-col gap-3 sm:flex-row">
          <Button
            variant="outline"
            className="w-full sm:flex-1"
            onClick={() => navigate("/teams")}
          >
            {t("join.viewTeams")}
          </Button>
          <Button
            className="w-full sm:flex-1"
            onClick={handleAccept}
            disabled={accepting}
          >
            {accepting ? (
              <>
                <Loader2 className="h-4 w-4 animate-spin" />
                {t("join.accepting")}
              </>
            ) : (
              <>
                <UserPlus className="h-4 w-4" />
                {t("join.acceptCta")}
              </>
            )}
          </Button>
        </CardFooter>
      </Card>
    );
  };

  return (
    <div className="min-h-screen bg-[linear-gradient(180deg,hsl(var(--background))_0%,hsl(var(--ui-shell-gradient-end))_100%),radial-gradient(circle_at_top_left,hsl(var(--primary)/0.11),transparent_34%),radial-gradient(circle_at_bottom_right,hsl(var(--status-success-text)/0.08),transparent_42%)]">
      <div className="mx-auto flex min-h-screen max-w-6xl flex-col px-4 py-6 sm:px-6 lg:px-8">
        <div className="flex items-center justify-between">
          <div>
            <div className="text-xs uppercase tracking-[0.24em] text-[hsl(var(--muted-foreground))]">
              {brand.name}
            </div>
            <div className="mt-2 flex items-center gap-3">
              <h1 className="font-display text-2xl font-semibold tracking-tight text-[hsl(var(--foreground))] sm:text-3xl">
                {t("join.title")}
              </h1>
              <Badge variant="secondary">{t("join.validBadge")}</Badge>
            </div>
          </div>
          <LanguageSwitcher />
        </div>

        <div className="mt-10 grid gap-6 lg:grid-cols-[1.15fr_0.85fr]">
          <Card className="ui-section-panel overflow-hidden">
            <CardHeader className="border-b border-[hsl(var(--border))/0.2] pb-6">
              <CardTitle>{t("join.subtitle")}</CardTitle>
              <CardDescription>{t("join.description")}</CardDescription>
            </CardHeader>
            <CardContent className="space-y-6 pt-6">
              <div className="grid gap-3 sm:grid-cols-2">
                <div className="rounded-2xl border border-[hsl(var(--border))/0.28] bg-[hsl(var(--muted))]/0.24 p-4">
                  <div className="flex items-center gap-2 text-xs uppercase tracking-[0.18em] text-[hsl(var(--muted-foreground))]">
                    <Users className="h-3.5 w-3.5" />
                    {t("join.teamLabel")}
                  </div>
                  <div className="mt-3 text-lg font-semibold">{teamName}</div>
                </div>
                <div className="rounded-2xl border border-[hsl(var(--border))/0.28] bg-[hsl(var(--muted))]/0.24 p-4">
                  <div className="text-xs uppercase tracking-[0.18em] text-[hsl(var(--muted-foreground))]">
                    {t("join.roleLabel")}
                  </div>
                  <div className="mt-3">
                    <Badge
                      variant={
                        invite?.role === "admin" ? "secondary" : "outline"
                      }
                    >
                      {invite?.role
                        ? t(`teams.roles.${invite.role}`)
                        : "member"}
                    </Badge>
                  </div>
                </div>
              </div>

              <div className="grid gap-3 sm:grid-cols-2">
                <div className="rounded-2xl border border-[hsl(var(--border))/0.28] bg-[hsl(var(--muted))]/0.24 p-4">
                  <div className="text-xs uppercase tracking-[0.18em] text-[hsl(var(--muted-foreground))]">
                    {t("join.expiresLabel")}
                  </div>
                  <div className="mt-3 text-sm font-medium text-[hsl(var(--foreground))]">
                    {invite?.expiresAt
                      ? formatDateTime(invite.expiresAt)
                      : t("join.noExpiry")}
                  </div>
                </div>
                <div className="rounded-2xl border border-[hsl(var(--border))/0.28] bg-[hsl(var(--muted))]/0.24 p-4">
                  <div className="text-xs uppercase tracking-[0.18em] text-[hsl(var(--muted-foreground))]">
                    {t("join.inviteeEmailLabel")}
                  </div>
                  <div className="mt-3 text-sm font-medium text-[hsl(var(--foreground))]">
                    {isOpenInvite
                      ? t("join.openInviteLabel")
                      : inviteeEmailHint || t("join.inviteeEmailHidden")}
                  </div>
                </div>
                <div className="rounded-2xl border border-[hsl(var(--border))/0.28] bg-[hsl(var(--muted))]/0.24 p-4">
                  <div className="text-xs uppercase tracking-[0.18em] text-[hsl(var(--muted-foreground))]">
                    {t("join.shareCode")}
                  </div>
                  <div className="mt-3 font-mono text-sm tracking-[0.14em] text-[hsl(var(--foreground))]">
                    {code}
                  </div>
                </div>
              </div>

              <div className="rounded-[22px] border border-dashed border-[hsl(var(--border))/0.34] bg-[hsl(var(--muted))]/0.18 p-5">
                <div className="text-sm font-medium">
                  {t("join.acceptHint")}
                </div>
                <p className="mt-2 max-w-xl text-sm leading-6 text-[hsl(var(--muted-foreground))]">
                  {t("join.description")}
                </p>
              </div>
            </CardContent>
          </Card>

          <div className="lg:pt-6">{renderActionCard()}</div>
        </div>
      </div>
    </div>
  );
}
