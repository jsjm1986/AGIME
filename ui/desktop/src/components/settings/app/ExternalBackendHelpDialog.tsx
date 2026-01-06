import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from '../../ui/dialog';
import { Tabs, TabsContent, TabsList, TabsTrigger } from '../../ui/tabs';
import { Button } from '../../ui/button';
import {
  HelpCircle,
  Server,
  Terminal,
  Settings,
  MessageCircleQuestion,
  Copy,
  Check,
} from 'lucide-react';

// 复制按钮
function CopyButton({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);

  const handleCopy = async (e: React.MouseEvent) => {
    e.stopPropagation();
    await navigator.clipboard.writeText(text);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  return (
    <button
      onClick={handleCopy}
      className="absolute top-3 right-3 p-1.5 rounded opacity-0 group-hover:opacity-100
                 hover:bg-white/10 transition-all"
      title="复制"
    >
      {copied ? (
        <Check className="w-4 h-4 text-teal-400" />
      ) : (
        <Copy className="w-4 h-4 text-gray-400" />
      )}
    </button>
  );
}

// 代码块 - 简洁风格
function CodeBlock({ children }: { children: string }) {
  return (
    <div className="relative group rounded-lg bg-[#1a1a1a] dark:bg-[#0d0d0d]">
      <pre className="p-4 text-[13px] leading-relaxed overflow-x-auto">
        <code className="text-gray-300 font-mono whitespace-pre">{children}</code>
      </pre>
      <CopyButton text={children} />
    </div>
  );
}

// 章节标题
function SectionTitle({ children }: { children: React.ReactNode }) {
  return (
    <h4 className="text-sm font-medium text-text-default mb-3 flex items-center gap-2">
      <span className="text-teal-500">›</span>
      {children}
    </h4>
  );
}

// 步骤项
function StepItem({ number, children }: { number: number; children: React.ReactNode }) {
  return (
    <div className="flex items-start gap-3">
      <span className="flex-shrink-0 w-5 h-5 rounded-full border border-teal-500/50
                       flex items-center justify-center text-xs text-teal-500">
        {number}
      </span>
      <span className="text-sm text-text-muted pt-0.5">{children}</span>
    </div>
  );
}

// 功能卡片
function FeatureCard({ title, desc }: { title: string; desc: string }) {
  return (
    <div className="rounded-lg border border-border-subtle p-4">
      <p className="text-sm font-medium text-text-default mb-1">{title}</p>
      <p className="text-xs text-text-muted leading-relaxed">{desc}</p>
    </div>
  );
}

// 概述选项卡
function OverviewTab() {
  const { t } = useTranslation('settings');

  return (
    <div className="space-y-6">
      <div>
        <SectionTitle>
          {t('session.agimeServer.help.overview.title', '什么是外部服务器？')}
        </SectionTitle>
        <p className="text-sm text-text-muted leading-relaxed">
          {t(
            'session.agimeServer.help.overview.description',
            'AGIME 默认会在本地启动一个后端服务器。使用外部服务器功能，您可以连接到运行在其他机器上的 AGIME 服务端，实现远程访问、负载分离或团队共享。'
          )}
        </p>
      </div>

      <div>
        <SectionTitle>{t('session.agimeServer.help.overview.useCases', '适用场景')}</SectionTitle>
        <div className="grid grid-cols-2 gap-3">
          <FeatureCard title="远程访问" desc="在高性能服务器上运行服务端，本地客户端连接使用" />
          <FeatureCard title="性能分离" desc="将 AI 推理负载放在专用服务器上" />
          <FeatureCard title="团队共享" desc="多人共用同一个后端服务，共享配置和扩展" />
          <FeatureCard title="无头服务器" desc="在没有图形界面的服务器上部署" />
        </div>
      </div>

      <div>
        <SectionTitle>
          {t('session.agimeServer.help.overview.howItWorks', '工作原理')}
        </SectionTitle>
        <div className="space-y-2.5">
          <StepItem number={1}>在服务器上启动 AGIME 服务端（agime agent 命令）</StepItem>
          <StepItem number={2}>在客户端设置中启用"使用外部服务器"</StepItem>
          <StepItem number={3}>输入服务器地址和认证密钥</StepItem>
          <StepItem number={4}>重启应用后，客户端将连接到远程服务器</StepItem>
        </div>
      </div>
    </div>
  );
}

// 服务端部署选项卡
function ServerDeploymentTab() {
  const { t } = useTranslation('settings');

  return (
    <div className="space-y-6">
      <div>
        <SectionTitle>
          {t('session.agimeServer.help.server.step1', '步骤 1：获取可执行文件')}
        </SectionTitle>
        <p className="text-sm text-text-muted mb-3">
          从 GitHub Release 下载对应平台的二进制文件
        </p>
        <CodeBlock>{`# Linux/macOS
wget https://github.com/jsjm1986/AGIME/releases/latest/download/agime-linux-x64
chmod +x agime-linux-x64
sudo mv agime-linux-x64 /usr/local/bin/agime`}</CodeBlock>
      </div>

      <div>
        <SectionTitle>
          {t('session.agimeServer.help.server.step2', '步骤 2：设置环境变量')}
        </SectionTitle>
        <CodeBlock>{`# 必须：设置认证密钥
export AGIME_SERVER__SECRET_KEY="your-secret-key"

# 可选：设置端口（默认 3000）
export AGIME_PORT=3000

# 必须：允许远程连接
export AGIME_SERVER__HOST=0.0.0.0

# 必须：设置 AI 提供商密钥
export ANTHROPIC_API_KEY="sk-ant-..."
# 或 export OPENAI_API_KEY="sk-..."`}</CodeBlock>
      </div>

      <div>
        <SectionTitle>
          {t('session.agimeServer.help.server.step3', '步骤 3：启动服务器')}
        </SectionTitle>
        <CodeBlock>agime agent</CodeBlock>
        <p className="text-sm text-text-muted mt-2">
          成功启动后显示：
          <code className="mx-1 px-1.5 py-0.5 rounded bg-background-muted text-xs">
            Server listening on 0.0.0.0:3000
          </code>
        </p>
      </div>

      <div>
        <SectionTitle>
          {t('session.agimeServer.help.server.systemd', '生产环境：Systemd 服务')}
        </SectionTitle>
        <p className="text-sm text-text-muted mb-3">
          创建 <code className="px-1.5 py-0.5 rounded bg-background-muted text-xs">/etc/systemd/system/agime.service</code>
        </p>
        <CodeBlock>{`[Unit]
Description=AGIME Server
After=network.target

[Service]
Type=simple
Environment="AGIME_SERVER__SECRET_KEY=your-secret-key"
Environment="AGIME_SERVER__HOST=0.0.0.0"
Environment="AGIME_PORT=3000"
Environment="ANTHROPIC_API_KEY=sk-ant-..."
ExecStart=/usr/local/bin/agime agent
Restart=always

[Install]
WantedBy=multi-user.target`}</CodeBlock>
        <div className="mt-3">
          <CodeBlock>{`sudo systemctl daemon-reload
sudo systemctl enable agime
sudo systemctl start agime`}</CodeBlock>
        </div>
      </div>

      <div>
        <SectionTitle>
          {t('session.agimeServer.help.server.verify', '验证部署')}
        </SectionTitle>
        <CodeBlock>{`curl http://localhost:3000/status
# 返回: {"status":"ok"}`}</CodeBlock>
      </div>
    </div>
  );
}

// 客户端配置选项卡
function ClientConfigTab() {
  const { t } = useTranslation('settings');

  return (
    <div className="space-y-6">
      <div>
        <SectionTitle>
          {t('session.agimeServer.help.client.step1', '步骤 1：启用外部服务器')}
        </SectionTitle>
        <div className="space-y-2">
          <StepItem number={1}>打开 AGIME 桌面应用</StepItem>
          <StepItem number={2}>进入 设置 → 会话 选项卡</StepItem>
          <StepItem number={3}>找到 AGIME 服务器 卡片</StepItem>
          <StepItem number={4}>开启"使用外部服务器"开关</StepItem>
        </div>
      </div>

      <div>
        <SectionTitle>
          {t('session.agimeServer.help.client.step2', '步骤 2：填写配置')}
        </SectionTitle>
        <div className="rounded-lg border border-border-subtle p-4 space-y-4">
          <div>
            <p className="text-sm font-medium text-text-default mb-1">服务器 URL</p>
            <p className="text-xs text-text-muted">
              格式：<code className="px-1.5 py-0.5 rounded bg-background-muted">http://服务器IP:端口</code>
            </p>
            <p className="text-xs text-text-muted mt-1">
              示例：<code className="px-1.5 py-0.5 rounded bg-background-muted text-teal-500">http://192.168.1.100:3000</code>
            </p>
          </div>
          <div className="border-t border-border-subtle pt-4">
            <p className="text-sm font-medium text-text-default mb-1">密钥</p>
            <p className="text-xs text-text-muted">
              与服务器端 <code className="px-1.5 py-0.5 rounded bg-background-muted">AGIME_SERVER__SECRET_KEY</code> 相同的值
            </p>
          </div>
        </div>
      </div>

      <div>
        <SectionTitle>
          {t('session.agimeServer.help.client.step3', '步骤 3：重启应用')}
        </SectionTitle>
        <p className="text-sm text-text-muted">
          配置会自动保存。完全退出并重启 AGIME 后，新的聊天窗口将连接到外部服务器。
        </p>
      </div>

      <div>
        <SectionTitle>{t('session.agimeServer.help.client.verify', '验证连接')}</SectionTitle>
        <p className="text-sm text-text-muted mb-3">
          如果连接失败，会弹出错误对话框：
        </p>
        <ul className="text-sm text-text-muted space-y-1.5">
          <li className="flex items-center gap-2">
            <span className="text-teal-500">•</span>
            禁用外部服务器并重试 - 自动回退到本地服务器
          </li>
          <li className="flex items-center gap-2">
            <span className="text-teal-500">•</span>
            退出 - 关闭应用，检查配置后重试
          </li>
        </ul>
      </div>
    </div>
  );
}

// 常见问题选项卡
function FAQTab() {
  const faqs = [
    {
      q: '连接失败怎么办？',
      a: '检查：1) 服务器是否运行；2) 防火墙是否开放端口；3) 密钥是否正确；4) URL 是否包含 http://',
    },
    {
      q: '可以使用 HTTPS 吗？',
      a: '可以。配置 SSL 证书后使用 https:// 协议。公网环境建议使用 HTTPS。',
    },
    {
      q: '密钥有什么要求？',
      a: '建议使用 32 位以上的随机字符串。可用 openssl rand -hex 32 生成。',
    },
    {
      q: '多个客户端可以连接同一服务器吗？',
      a: '可以。使用相同密钥即可同时连接。',
    },
    {
      q: '外部服务器需要 GPU 吗？',
      a: '不需要。AGIME 只是中间层，调用外部 AI API 进行推理。',
    },
    {
      q: '如何切回本地服务器？',
      a: '关闭"使用外部服务器"开关，重启应用即可。',
    },
  ];

  return (
    <div className="space-y-4">
      {faqs.map((faq, index) => (
        <div key={index} className="pb-4 border-b border-border-subtle last:border-0 last:pb-0">
          <p className="text-sm font-medium text-text-default mb-1.5">{faq.q}</p>
          <p className="text-sm text-text-muted leading-relaxed">{faq.a}</p>
        </div>
      ))}
    </div>
  );
}

// 主组件
export default function ExternalBackendHelpDialog() {
  const { t } = useTranslation('settings');
  const [open, setOpen] = useState(false);

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogTrigger asChild>
        <Button
          variant="ghost"
          size="sm"
          className="h-6 w-6 p-0 text-text-muted hover:text-teal-500 transition-colors"
          title={t('session.agimeServer.help.title', '使用帮助')}
        >
          <HelpCircle className="h-4 w-4" />
        </Button>
      </DialogTrigger>
      <DialogContent className="max-w-2xl max-h-[80vh] flex flex-col p-0">
        <DialogHeader className="px-6 py-5 border-b border-border-subtle flex-shrink-0">
          <DialogTitle className="flex items-center gap-2.5 text-base">
            <Server className="h-5 w-5 text-text-muted" />
            {t('session.agimeServer.help.dialogTitle', 'AGIME 外部服务器使用指南')}
          </DialogTitle>
        </DialogHeader>

        <Tabs defaultValue="overview" className="flex-1 flex flex-col overflow-hidden">
          <div className="px-6 pt-4 flex-shrink-0">
            <TabsList className="w-full justify-start gap-4 bg-transparent p-0 h-auto">
              <TabsTrigger
                value="overview"
                className="px-0 py-2 text-sm bg-transparent border-b-2 border-transparent rounded-none
                           data-[state=active]:border-teal-500 data-[state=active]:text-teal-500
                           data-[state=active]:bg-transparent data-[state=active]:shadow-none
                           hover:text-text-default transition-colors"
              >
                <HelpCircle className="h-4 w-4 mr-1.5" />
                {t('session.agimeServer.help.tabs.overview', '概述')}
              </TabsTrigger>
              <TabsTrigger
                value="server"
                className="px-0 py-2 text-sm bg-transparent border-b-2 border-transparent rounded-none
                           data-[state=active]:border-teal-500 data-[state=active]:text-teal-500
                           data-[state=active]:bg-transparent data-[state=active]:shadow-none
                           hover:text-text-default transition-colors"
              >
                <Terminal className="h-4 w-4 mr-1.5" />
                {t('session.agimeServer.help.tabs.server', '服务端部署')}
              </TabsTrigger>
              <TabsTrigger
                value="client"
                className="px-0 py-2 text-sm bg-transparent border-b-2 border-transparent rounded-none
                           data-[state=active]:border-teal-500 data-[state=active]:text-teal-500
                           data-[state=active]:bg-transparent data-[state=active]:shadow-none
                           hover:text-text-default transition-colors"
              >
                <Settings className="h-4 w-4 mr-1.5" />
                {t('session.agimeServer.help.tabs.client', '客户端配置')}
              </TabsTrigger>
              <TabsTrigger
                value="faq"
                className="px-0 py-2 text-sm bg-transparent border-b-2 border-transparent rounded-none
                           data-[state=active]:border-teal-500 data-[state=active]:text-teal-500
                           data-[state=active]:bg-transparent data-[state=active]:shadow-none
                           hover:text-text-default transition-colors"
              >
                <MessageCircleQuestion className="h-4 w-4 mr-1.5" />
                {t('session.agimeServer.help.tabs.faq', '常见问题')}
              </TabsTrigger>
            </TabsList>
          </div>

          <div className="flex-1 overflow-y-auto px-6 py-5">
            <TabsContent value="overview" className="mt-0 focus-visible:outline-none">
              <OverviewTab />
            </TabsContent>
            <TabsContent value="server" className="mt-0 focus-visible:outline-none">
              <ServerDeploymentTab />
            </TabsContent>
            <TabsContent value="client" className="mt-0 focus-visible:outline-none">
              <ClientConfigTab />
            </TabsContent>
            <TabsContent value="faq" className="mt-0 focus-visible:outline-none">
              <FAQTab />
            </TabsContent>
          </div>
        </Tabs>
      </DialogContent>
    </Dialog>
  );
}
