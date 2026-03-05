import { useTranslation } from 'react-i18next';
import { Bot, ClipboardList, ShieldCheck, Sparkles, UserRound } from 'lucide-react';
import { Card, CardContent, CardHeader, CardTitle } from '../../ui/card';
import { Button } from '../../ui/button';

interface DigitalAvatarGuideProps {
  canSendCommand?: boolean;
  onCopyCommand?: (text: string) => void;
  onSendCommand?: (text: string) => void;
}

export function DigitalAvatarGuide({
  canSendCommand = false,
  onCopyCommand,
  onSendCommand,
}: DigitalAvatarGuideProps) {
  const { t } = useTranslation();

  const roleCards = [
    {
      icon: <UserRound className="h-4 w-4" />,
      title: t('digitalAvatar.guide.roleBusinessTitle', { defaultValue: '你（业务负责人）' }),
      desc: t('digitalAvatar.guide.roleBusinessDesc', {
        defaultValue: '负责给目标、看结果、做最终决策。你只需要描述业务需求，不需要手动改一堆技术配置。',
      }),
    },
    {
      icon: <Bot className="h-4 w-4" />,
      title: t('digitalAvatar.guide.roleManagerTitle', { defaultValue: '管理 Agent（分身经理）' }),
      desc: t('digitalAvatar.guide.roleManagerDesc', {
        defaultValue: '负责创建/调整分身、判断是否提权、产出优化建议。它是你的“运营中控”。',
      }),
    },
    {
      icon: <Sparkles className="h-4 w-4" />,
      title: t('digitalAvatar.guide.roleServiceTitle', { defaultValue: '分身 Agent（执行者）' }),
      desc: t('digitalAvatar.guide.roleServiceDesc', {
        defaultValue: '负责具体执行任务与对外服务。能力不足时会上报管理 Agent，不会直接越权。',
      }),
    },
  ];

  const steps = [
    {
      title: t('digitalAvatar.guide.step1Title'),
      desc: t('digitalAvatar.guide.step1Desc'),
    },
    {
      title: t('digitalAvatar.guide.step2Title'),
      desc: t('digitalAvatar.guide.step2Desc'),
    },
    {
      title: t('digitalAvatar.guide.step3Title'),
      desc: t('digitalAvatar.guide.step3Desc'),
    },
    {
      title: t('digitalAvatar.guide.step4Title'),
      desc: t('digitalAvatar.guide.step4Desc'),
    },
    {
      title: t('digitalAvatar.guide.step5Title'),
      desc: t('digitalAvatar.guide.step5Desc'),
    },
  ];

  const quickCommands = [
    {
      id: 'cmd1',
      text: t('digitalAvatar.guide.quickCommand1', {
        defaultValue: '请先根据我的目标，创建一个新的对外数字分身，并给出最小权限配置。',
      }),
    },
    {
      id: 'cmd2',
      text: t('digitalAvatar.guide.quickCommand2', {
        defaultValue: '请检查这个分身最近失败原因，给我3条最小改动建议，并标注风险。',
      }),
    },
    {
      id: 'cmd3',
      text: t('digitalAvatar.guide.quickCommand3', {
        defaultValue: '如果现有能力不够，请先给提权建议，再给临时授权范围和回滚方案。',
      }),
    },
    {
      id: 'cmd4',
      text: t('digitalAvatar.guide.quickCommand4', {
        defaultValue: '如果这个能力缺口经常出现，请提交新增分身提案，并说明预期收益。',
      }),
    },
  ] as const;

  const quickActionsTitle = t('digitalAvatar.guide.quickActionsTitle', {
    defaultValue: '快捷操作',
  });

  const copyText = t('digitalAvatar.guide.copy', { defaultValue: '复制' });
  const sendText = t('digitalAvatar.guide.sendToManager', { defaultValue: '发给管理 Agent' });

  const approvalBoundaries = [
    t('digitalAvatar.guide.boundary1', {
      defaultValue: '涉及新增文档写入权限、扩展权限、技能权限时，建议走人工确认。',
    }),
    t('digitalAvatar.guide.boundary2', {
      defaultValue: '涉及对外公开发布、权限放大、跨团队访问时，必须人工确认。',
    }),
    t('digitalAvatar.guide.boundary3', {
      defaultValue: '管理 Agent 可以先给“最小临时授权”，验证通过后再正式放开。',
    }),
  ];

  const noManagerHint = t('digitalAvatar.guide.noManagerHint', {
    defaultValue: '当前未绑定管理 Agent，可先复制指令，绑定后再发送执行。',
  });

  return (
    <div className="space-y-4 p-4 sm:p-5">
      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-sm">{t('digitalAvatar.guide.title')}</CardTitle>
          <p className="text-caption text-muted-foreground">{t('digitalAvatar.guide.subtitle')}</p>
          <p className="text-caption text-muted-foreground">
            {t('digitalAvatar.guide.whatsDifferent', {
              defaultValue: '核心理念：你主要和管理 Agent 对话，由管理 Agent 协调“分身创建、治理审批、持续优化”。',
            })}
          </p>
        </CardHeader>
        <CardContent className="space-y-3 text-xs">
          <div className="grid gap-2 sm:grid-cols-3">
            {roleCards.map((role) => (
              <div key={role.title} className="rounded-md border border-border/60 bg-muted/20 p-3">
                <p className="flex items-center gap-1.5 font-medium text-foreground">
                  {role.icon}
                  {role.title}
                </p>
                <p className="mt-1 text-muted-foreground">{role.desc}</p>
              </div>
            ))}
          </div>

          <div className="rounded-md border border-border/60 bg-background p-3 text-muted-foreground">
            <p className="font-medium text-foreground">
              {t('digitalAvatar.guide.oneLineFlowTitle', { defaultValue: '一句话流程' })}
            </p>
            <p className="mt-1">
              {t('digitalAvatar.guide.oneLineFlow', {
                defaultValue:
                  '你提目标 -> 管理 Agent 规划与配置 -> 分身 Agent 执行交付 -> 管理 Agent 复盘并持续优化。',
              })}
            </p>
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-sm flex items-center gap-1.5">
            <ClipboardList className="h-4 w-4" />
            {t('digitalAvatar.guide.quickStartTitle', { defaultValue: '5 分钟上手（推荐顺序）' })}
          </CardTitle>
        </CardHeader>
        <CardContent className="space-y-3 text-xs">
          {steps.map((step, idx) => (
            <div key={step.title} className="rounded-md border border-border/50 bg-muted/20 p-3">
              <p className="font-medium text-foreground">
                {idx + 1}. {step.title}
              </p>
              <p className="mt-1 text-muted-foreground">{step.desc}</p>
            </div>
          ))}
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-sm">{t('digitalAvatar.guide.suggestionsTitle')}</CardTitle>
          <p className="text-caption text-muted-foreground">
            {t('digitalAvatar.guide.suggestionsHint', {
              defaultValue: '下面这几句可以直接发给管理 Agent，适合小白快速开始。',
            })}
          </p>
        </CardHeader>
        <CardContent className="space-y-2 text-xs text-muted-foreground">
          {!canSendCommand && (
            <p className="rounded-md border border-status-warning/35 bg-status-warning/10 p-2 text-[11px] text-status-warning-text">
              {noManagerHint}
            </p>
          )}
          {quickCommands.map((item) => (
            <div key={item.id} className="rounded-md border border-dashed border-border/60 bg-background p-2.5">
              <p>{item.text}</p>
              <div className="mt-2 flex items-center gap-1.5">
                <Button
                  size="sm"
                  variant="outline"
                  className="h-7 px-2 text-[11px]"
                  onClick={() => onCopyCommand?.(item.text)}
                >
                  {copyText}
                </Button>
                <Button
                  size="sm"
                  className="h-7 px-2 text-[11px]"
                  onClick={() => onSendCommand?.(item.text)}
                  disabled={!canSendCommand}
                >
                  {sendText}
                </Button>
              </div>
            </div>
          ))}
          <div className="rounded-md border border-border/60 bg-muted/20 p-2 text-[11px]">
            <span className="font-medium text-foreground">{quickActionsTitle}：</span>{' '}
            {t('digitalAvatar.guide.quickActionsHint', {
              defaultValue: '复制用于保存模板；发送用于立即让管理 Agent 执行。',
            })}
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-sm flex items-center gap-1.5">
            <ShieldCheck className="h-4 w-4" />
            {t('digitalAvatar.guide.approvalTitle', { defaultValue: '什么时候需要人工确认' })}
          </CardTitle>
        </CardHeader>
        <CardContent className="space-y-2 text-xs text-muted-foreground">
          {approvalBoundaries.map((item) => (
            <div key={item} className="rounded-md border border-border/60 bg-muted/20 p-2.5">
              {item}
            </div>
          ))}
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-sm">{t('digitalAvatar.guide.faqTitle')}</CardTitle>
        </CardHeader>
        <CardContent className="space-y-3 text-xs">
          <div>
            <p className="font-medium">{t('digitalAvatar.guide.faq1Q')}</p>
            <p className="mt-1 text-muted-foreground">{t('digitalAvatar.guide.faq1A')}</p>
          </div>
          <div>
            <p className="font-medium">{t('digitalAvatar.guide.faq2Q')}</p>
            <p className="mt-1 text-muted-foreground">{t('digitalAvatar.guide.faq2A')}</p>
          </div>
          <div>
            <p className="font-medium">{t('digitalAvatar.guide.faq3Q')}</p>
            <p className="mt-1 text-muted-foreground">{t('digitalAvatar.guide.faq3A')}</p>
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
