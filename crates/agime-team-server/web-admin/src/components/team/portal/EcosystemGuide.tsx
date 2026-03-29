import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Bot, ClipboardList, Copy, FileText, Globe, ShieldCheck, UserRound } from 'lucide-react';
import { Card, CardContent, CardHeader, CardTitle } from '../../ui/card';
import { Button } from '../../ui/button';

export function EcosystemGuide() {
  const { t } = useTranslation();
  const [copiedId, setCopiedId] = useState<string | null>(null);

  const roles = [
    {
      icon: <UserRound className="h-4 w-4" />,
      title: t('ecosystem.guide.roleBusinessTitle'),
      desc: t('ecosystem.guide.roleBusinessDesc'),
    },
    {
      icon: <Bot className="h-4 w-4" />,
      title: t('ecosystem.guide.roleCodingTitle'),
      desc: t('ecosystem.guide.roleCodingDesc'),
    },
    {
      icon: <Globe className="h-4 w-4" />,
      title: t('ecosystem.guide.roleServiceTitle'),
      desc: t('ecosystem.guide.roleServiceDesc'),
    },
  ];

  const steps = [
    { title: t('ecosystem.guide.step1Title'), desc: t('ecosystem.guide.step1Desc') },
    { title: t('ecosystem.guide.step2Title'), desc: t('ecosystem.guide.step2Desc') },
    { title: t('ecosystem.guide.step3Title'), desc: t('ecosystem.guide.step3Desc') },
    { title: t('ecosystem.guide.step4Title'), desc: t('ecosystem.guide.step4Desc') },
    { title: t('ecosystem.guide.step5Title'), desc: t('ecosystem.guide.step5Desc') },
  ];

  const commands = [
    { id: 'cmd1', text: t('ecosystem.guide.command1') },
    { id: 'cmd2', text: t('ecosystem.guide.command2') },
    { id: 'cmd3', text: t('ecosystem.guide.command3') },
    { id: 'cmd4', text: t('ecosystem.guide.command4') },
  ];

  const copyCommand = async (id: string, text: string) => {
    try {
      await navigator.clipboard.writeText(text);
      setCopiedId(id);
      window.setTimeout(() => setCopiedId(null), 1400);
    } catch {
      setCopiedId(null);
    }
  };

  return (
    <div className="space-y-4 p-4 sm:p-5">
      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-sm">{t('ecosystem.guide.title')}</CardTitle>
          <p className="text-caption text-muted-foreground">{t('ecosystem.guide.subtitle')}</p>
        </CardHeader>
        <CardContent className="space-y-3 text-xs">
          <div className="rounded-md border border-border/60 bg-background p-3 text-muted-foreground">
            <p className="font-medium text-foreground">{t('ecosystem.guide.whatIsTitle')}</p>
            <p className="mt-1">{t('ecosystem.guide.whatIsDesc')}</p>
          </div>
          <div className="grid gap-2 sm:grid-cols-3">
            {roles.map((role) => (
              <div key={role.title} className="rounded-md border border-border/60 bg-muted/20 p-3">
                <p className="flex items-center gap-1.5 font-medium text-foreground">
                  {role.icon}
                  {role.title}
                </p>
                <p className="mt-1 text-muted-foreground">{role.desc}</p>
              </div>
            ))}
          </div>
          <div className="rounded-md border border-border/60 bg-muted/20 p-3 text-muted-foreground">
            <p className="font-medium text-foreground">{t('ecosystem.guide.flowTitle')}</p>
            <p className="mt-1">{t('ecosystem.guide.flowDesc')}</p>
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-sm flex items-center gap-1.5">
            <ClipboardList className="h-4 w-4" />
            {t('ecosystem.guide.quickStartTitle')}
          </CardTitle>
        </CardHeader>
        <CardContent className="space-y-3 text-xs">
          {steps.map((step, idx) => (
            <div key={step.title} className="rounded-md border border-border/60 bg-muted/20 p-3">
              <p className="font-medium text-foreground">{idx + 1}. {step.title}</p>
              <p className="mt-1 text-muted-foreground">{step.desc}</p>
            </div>
          ))}
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-sm flex items-center gap-1.5">
            <ShieldCheck className="h-4 w-4" />
            {t('ecosystem.guide.permissionTitle')}
          </CardTitle>
        </CardHeader>
        <CardContent className="space-y-2 text-xs text-muted-foreground">
          <div className="rounded-md border border-border/60 bg-muted/20 p-2.5">
            <p className="font-medium text-foreground">{t('ecosystem.documentAccessModeReadOnly')}</p>
            <p className="mt-1">{t('ecosystem.guide.permissionReadOnly')}</p>
          </div>
          <div className="rounded-md border border-border/60 bg-muted/20 p-2.5">
            <p className="font-medium text-foreground">{t('ecosystem.documentAccessModeCoEditDraft')}</p>
            <p className="mt-1">{t('ecosystem.guide.permissionCoEdit')}</p>
          </div>
          <div className="rounded-md border border-border/60 bg-muted/20 p-2.5">
            <p className="font-medium text-foreground">{t('ecosystem.documentAccessModeControlledWrite')}</p>
            <p className="mt-1">{t('ecosystem.guide.permissionControlled')}</p>
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-sm flex items-center gap-1.5">
            <FileText className="h-4 w-4" />
            {t('ecosystem.guide.commandsTitle')}
          </CardTitle>
          <p className="text-caption text-muted-foreground">{t('ecosystem.guide.commandsHint')}</p>
        </CardHeader>
        <CardContent className="space-y-2 text-xs text-muted-foreground">
          {commands.map((item) => (
            <div key={item.id} className="rounded-md border border-dashed border-border/60 bg-background p-2.5">
              <p>{item.text}</p>
              <div className="mt-2">
                <Button
                  size="sm"
                  variant="outline"
                  className="h-7 px-2 text-[11px]"
                  onClick={() => copyCommand(item.id, item.text)}
                >
                  <Copy className="mr-1 h-3 w-3" />
                  {copiedId === item.id ? t('ecosystem.copiedUrl') : t('ecosystem.guide.copy')}
                </Button>
              </div>
            </div>
          ))}
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-sm">{t('ecosystem.guide.faqTitle')}</CardTitle>
        </CardHeader>
        <CardContent className="space-y-3 text-xs">
          <div>
            <p className="font-medium">{t('ecosystem.guide.faq1Q')}</p>
            <p className="mt-1 text-muted-foreground">{t('ecosystem.guide.faq1A')}</p>
          </div>
          <div>
            <p className="font-medium">{t('ecosystem.guide.faq2Q')}</p>
            <p className="mt-1 text-muted-foreground">{t('ecosystem.guide.faq2A')}</p>
          </div>
          <div>
            <p className="font-medium">{t('ecosystem.guide.faq3Q')}</p>
            <p className="mt-1 text-muted-foreground">{t('ecosystem.guide.faq3A')}</p>
          </div>
        </CardContent>
      </Card>
    </div>
  );
}

