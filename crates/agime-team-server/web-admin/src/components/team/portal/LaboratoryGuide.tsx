import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Bot, ClipboardList, Copy, FileText, Globe, ShieldCheck, UserRound } from 'lucide-react';
import { Card, CardContent, CardHeader, CardTitle } from '../../ui/card';
import { Button } from '../../ui/button';

export function LaboratoryGuide() {
  const { t } = useTranslation();
  const [copiedId, setCopiedId] = useState<string | null>(null);

  const roles = [
    {
      icon: <UserRound className="h-4 w-4" />,
      title: t('laboratory.guide.roleBusinessTitle'),
      desc: t('laboratory.guide.roleBusinessDesc'),
    },
    {
      icon: <Bot className="h-4 w-4" />,
      title: t('laboratory.guide.roleCodingTitle'),
      desc: t('laboratory.guide.roleCodingDesc'),
    },
    {
      icon: <Globe className="h-4 w-4" />,
      title: t('laboratory.guide.roleServiceTitle'),
      desc: t('laboratory.guide.roleServiceDesc'),
    },
  ];

  const steps = [
    { title: t('laboratory.guide.step1Title'), desc: t('laboratory.guide.step1Desc') },
    { title: t('laboratory.guide.step2Title'), desc: t('laboratory.guide.step2Desc') },
    { title: t('laboratory.guide.step3Title'), desc: t('laboratory.guide.step3Desc') },
    { title: t('laboratory.guide.step4Title'), desc: t('laboratory.guide.step4Desc') },
    { title: t('laboratory.guide.step5Title'), desc: t('laboratory.guide.step5Desc') },
  ];

  const commands = [
    { id: 'cmd1', text: t('laboratory.guide.command1') },
    { id: 'cmd2', text: t('laboratory.guide.command2') },
    { id: 'cmd3', text: t('laboratory.guide.command3') },
    { id: 'cmd4', text: t('laboratory.guide.command4') },
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
          <CardTitle className="text-sm">{t('laboratory.guide.title')}</CardTitle>
          <p className="text-caption text-muted-foreground">{t('laboratory.guide.subtitle')}</p>
        </CardHeader>
        <CardContent className="space-y-3 text-xs">
          <div className="rounded-md border border-border/60 bg-background p-3 text-muted-foreground">
            <p className="font-medium text-foreground">{t('laboratory.guide.whatIsTitle')}</p>
            <p className="mt-1">{t('laboratory.guide.whatIsDesc')}</p>
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
            <p className="font-medium text-foreground">{t('laboratory.guide.flowTitle')}</p>
            <p className="mt-1">{t('laboratory.guide.flowDesc')}</p>
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-sm flex items-center gap-1.5">
            <ClipboardList className="h-4 w-4" />
            {t('laboratory.guide.quickStartTitle')}
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
            {t('laboratory.guide.permissionTitle')}
          </CardTitle>
        </CardHeader>
        <CardContent className="space-y-2 text-xs text-muted-foreground">
          <div className="rounded-md border border-border/60 bg-muted/20 p-2.5">
            <p className="font-medium text-foreground">{t('laboratory.documentAccessModeReadOnly')}</p>
            <p className="mt-1">{t('laboratory.guide.permissionReadOnly')}</p>
          </div>
          <div className="rounded-md border border-border/60 bg-muted/20 p-2.5">
            <p className="font-medium text-foreground">{t('laboratory.documentAccessModeCoEditDraft')}</p>
            <p className="mt-1">{t('laboratory.guide.permissionCoEdit')}</p>
          </div>
          <div className="rounded-md border border-border/60 bg-muted/20 p-2.5">
            <p className="font-medium text-foreground">{t('laboratory.documentAccessModeControlledWrite')}</p>
            <p className="mt-1">{t('laboratory.guide.permissionControlled')}</p>
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-sm flex items-center gap-1.5">
            <FileText className="h-4 w-4" />
            {t('laboratory.guide.commandsTitle')}
          </CardTitle>
          <p className="text-caption text-muted-foreground">{t('laboratory.guide.commandsHint')}</p>
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
                  {copiedId === item.id ? t('laboratory.copiedUrl') : t('laboratory.guide.copy')}
                </Button>
              </div>
            </div>
          ))}
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-sm">{t('laboratory.guide.faqTitle')}</CardTitle>
        </CardHeader>
        <CardContent className="space-y-3 text-xs">
          <div>
            <p className="font-medium">{t('laboratory.guide.faq1Q')}</p>
            <p className="mt-1 text-muted-foreground">{t('laboratory.guide.faq1A')}</p>
          </div>
          <div>
            <p className="font-medium">{t('laboratory.guide.faq2Q')}</p>
            <p className="mt-1 text-muted-foreground">{t('laboratory.guide.faq2A')}</p>
          </div>
          <div>
            <p className="font-medium">{t('laboratory.guide.faq3Q')}</p>
            <p className="mt-1 text-muted-foreground">{t('laboratory.guide.faq3A')}</p>
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
