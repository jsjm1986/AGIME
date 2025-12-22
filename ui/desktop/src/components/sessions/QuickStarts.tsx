import { useTranslation } from 'react-i18next';
import { cn } from '../../utils';
import {
  Brain,
  BarChart3,
  Code2,
  Sparkles,
  Download,
  Settings2,
  Lightbulb,
  UserCircle,
} from 'lucide-react';

export interface QuickStartItem {
  id: string;
  icon: React.ReactNode;
  title: string;
  description: string;
  prompt: string;
  category: 'memory' | 'visualization' | 'development' | 'extension' | 'tutorial';
}

const quickStartItems: QuickStartItem[] = [
  // Memory & Personal Assistant
  {
    id: 'remember-me',
    icon: <UserCircle className="w-4 h-4" />,
    title: '记住我的信息',
    description: '让 AI 记住你的个人偏好',
    prompt: '记住我的信息：我叫[你的名字]，[你的职业]，常用技术栈是[技术栈]。以后的对话中请记住这些信息。',
    category: 'memory',
  },
  {
    id: 'show-capabilities',
    icon: <Sparkles className="w-4 h-4" />,
    title: '探索全部能力',
    description: '了解 AI 助手能帮你做什么',
    prompt: '你都能做什么？请详细介绍你的全部能力，包括可用的工具和扩展功能。',
    category: 'tutorial',
  },
  // Data Visualization
  {
    id: 'data-viz-mode',
    icon: <BarChart3 className="w-4 h-4" />,
    title: '数据可视化模式',
    description: '专注于数据分析和图表',
    prompt: '我需要进行数据可视化分析，请帮我：1) 启用 Auto Visualiser 扩展 2) 关闭其他不需要的扩展。然后告诉我如何开始分析数据。',
    category: 'visualization',
  },
  {
    id: 'analyze-data',
    icon: <Lightbulb className="w-4 h-4" />,
    title: '智能数据分析',
    description: '分析 CSV/JSON 数据',
    prompt: '请帮我分析数据文件，生成统计摘要和可视化图表。我会提供数据文件路径。',
    category: 'visualization',
  },
  // Development
  {
    id: 'analyze-project',
    icon: <Code2 className="w-4 h-4" />,
    title: '项目代码分析',
    description: '理解项目结构和代码',
    prompt: '请分析当前项目的代码结构，包括：1) 目录结构 2) 主要技术栈 3) 核心模块功能 4) 代码组织方式。',
    category: 'development',
  },
  {
    id: 'generate-readme',
    icon: <Brain className="w-4 h-4" />,
    title: '生成项目文档',
    description: '自动生成 README',
    prompt: '请根据当前项目代码，生成一份完整的 README.md 文档，包括项目介绍、安装步骤、使用方法和 API 说明。',
    category: 'development',
  },
  // Extension Management
  {
    id: 'install-skills',
    icon: <Download className="w-4 h-4" />,
    title: '安装 Skills 扩展',
    description: '从 GitHub 获取新能力',
    prompt: '请从 https://github.com/anthropics/skills 下载并安装最新的 skills 扩展包，然后告诉我新增了哪些功能。',
    category: 'extension',
  },
  {
    id: 'manage-extensions',
    icon: <Settings2 className="w-4 h-4" />,
    title: '扩展管理',
    description: '查看和管理已安装扩展',
    prompt: '请显示我当前已安装的所有扩展及其状态，并说明每个扩展的主要功能。',
    category: 'extension',
  },
];

const categoryColors: Record<QuickStartItem['category'], string> = {
  memory: 'bg-purple-200 hover:bg-purple-300 dark:bg-purple-500/15 dark:hover:bg-purple-500/25',
  visualization: 'bg-cyan-200 hover:bg-cyan-300 dark:bg-cyan-500/15 dark:hover:bg-cyan-500/25',
  development: 'bg-emerald-200 hover:bg-emerald-300 dark:bg-emerald-500/15 dark:hover:bg-emerald-500/25',
  extension: 'bg-orange-200 hover:bg-orange-300 dark:bg-orange-500/15 dark:hover:bg-orange-500/25',
  tutorial: 'bg-pink-200 hover:bg-pink-300 dark:bg-pink-500/15 dark:hover:bg-pink-500/25',
};

const categoryIconColors: Record<QuickStartItem['category'], string> = {
  memory: 'text-purple-600 dark:text-purple-400',
  visualization: 'text-cyan-600 dark:text-cyan-400',
  development: 'text-green-600 dark:text-green-400',
  extension: 'text-orange-600 dark:text-orange-400',
  tutorial: 'text-pink-600 dark:text-pink-400',
};

interface QuickStartsProps {
  onSelectPrompt: (prompt: string) => void;
}

export function QuickStarts({ onSelectPrompt }: QuickStartsProps) {
  const { t } = useTranslation('sessions');

  return (
    <div className="pb-2">
      <div className="mb-2">
        <h3 className="text-sm font-medium text-text-muted">
          {t('quickStarts.title', '快速开始')}
        </h3>
      </div>

      <div className="grid grid-cols-2 md:grid-cols-4 gap-2">
        {quickStartItems.map((item, index) => (
          <button
            key={item.id}
            onClick={() => onSelectPrompt(item.prompt)}
            className={cn(
              "group relative flex flex-col items-start p-3 rounded-lg",
              "border border-black/10 dark:border-white/10",
              "transition-all duration-200 ease-out",
              "hover:scale-[1.02] hover:shadow-lg",
              "text-left cursor-pointer",
              "animate-card-entrance",
              categoryColors[item.category]
            )}
            style={{ animationDelay: `${0.05 + index * 0.02}s` }}
          >
            {/* Icon */}
            <div className={cn(
              "mb-2 p-1.5 rounded-md bg-white/80 dark:bg-white/5",
              "group-hover:bg-white dark:group-hover:bg-white/10 transition-colors",
              "shadow-sm",
              categoryIconColors[item.category]
            )}>
              {item.icon}
            </div>

            {/* Title */}
            <h4 className="text-xs font-medium text-text-default mb-0.5 line-clamp-1">
              {item.title}
            </h4>

            {/* Description */}
            <p className="text-[11px] text-text-muted line-clamp-1">
              {item.description}
            </p>
          </button>
        ))}
      </div>
    </div>
  );
}
