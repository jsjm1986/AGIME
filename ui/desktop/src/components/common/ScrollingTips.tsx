import { useState, useEffect, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { cn } from '../../utils';
import {
  Lightbulb,
  FolderOpen,
  Brain,
  FileText,
  BarChart3,
  Mail,
  Camera,
  Sparkles,
  Coffee,
  Gift,
  Briefcase,
  GraduationCap,
  Heart,
  Zap,
  Search,
  Globe,
  Calculator,
  MessageSquare,
  Clock,
  Palette,
  HardDrive,
  Trash2,
  Shield,
  Wifi,
  Monitor,
  Package,
  RefreshCw,
} from 'lucide-react';

export interface Tip {
  icon: React.ReactNode;
  category: string;
  text: string;
  prompt: string;
}

export const tips: Tip[] = [
  // 电脑清理与系统维护
  { icon: <HardDrive className="w-3.5 h-3.5" />, category: '清理', text: 'C盘快满了？', prompt: '帮我检查一下C盘的缓存文件，看看哪些可以清理' },
  { icon: <Trash2 className="w-3.5 h-3.5" />, category: '清理', text: '想清理垃圾？', prompt: '帮我清理电脑上的临时文件和缓存' },
  { icon: <Package className="w-3.5 h-3.5" />, category: '清理', text: '软件装太多？', prompt: '帮我找出长期不用的软件，我考虑卸载' },
  { icon: <HardDrive className="w-3.5 h-3.5" />, category: '清理', text: '大文件在哪？', prompt: '扫描一下哪些文件最占空间，列出来给我看' },
  { icon: <Trash2 className="w-3.5 h-3.5" />, category: '清理', text: '重复文件太多？', prompt: '帮我找出电脑里的重复文件，我要清理' },
  { icon: <RefreshCw className="w-3.5 h-3.5" />, category: '清理', text: '浏览器卡顿？', prompt: '帮我清理浏览器缓存和历史记录' },
  { icon: <HardDrive className="w-3.5 h-3.5" />, category: '清理', text: '回收站忘清？', prompt: '帮我清空回收站，释放磁盘空间' },
  { icon: <Package className="w-3.5 h-3.5" />, category: '清理', text: '开机太慢？', prompt: '帮我看看有哪些开机启动项可以禁用' },

  // 系统诊断
  { icon: <Monitor className="w-3.5 h-3.5" />, category: '系统', text: '电脑突然变慢？', prompt: '帮我检查一下是什么在占用CPU和内存' },
  { icon: <Shield className="w-3.5 h-3.5" />, category: '系统', text: '担心有病毒？', prompt: '帮我扫描一下电脑有没有可疑程序' },
  { icon: <Wifi className="w-3.5 h-3.5" />, category: '系统', text: '网速不稳定？', prompt: '帮我测试一下网络速度和连接状态' },
  { icon: <Monitor className="w-3.5 h-3.5" />, category: '系统', text: '蓝屏怎么办？', prompt: '电脑蓝屏了，帮我分析一下可能的原因' },
  { icon: <HardDrive className="w-3.5 h-3.5" />, category: '系统', text: '硬盘健康吗？', prompt: '帮我检查一下硬盘的健康状态' },

  // 文件整理
  { icon: <FolderOpen className="w-3.5 h-3.5" />, category: '整理', text: '桌面乱成一锅粥？', prompt: '帮我把桌面文件按类型整理到不同文件夹' },
  { icon: <FolderOpen className="w-3.5 h-3.5" />, category: '整理', text: '下载文件夹爆炸了？', prompt: '清理下载文件夹，按日期和类型归类' },
  { icon: <FolderOpen className="w-3.5 h-3.5" />, category: '整理', text: '照片太多找不到？', prompt: '把相册里的照片按年份和月份整理好' },

  // 记忆管理
  { icon: <Brain className="w-3.5 h-3.5" />, category: '记忆', text: '换工作了？', prompt: '更新一下，我现在在新公司做产品经理了' },
  { icon: <Brain className="w-3.5 h-3.5" />, category: '记忆', text: '口味变了？', prompt: '忘掉之前的，我现在不喜欢吃辣了' },
  { icon: <Brain className="w-3.5 h-3.5" />, category: '记忆', text: '想看存了啥？', prompt: '展示一下你记住的关于我的所有信息' },
  { icon: <Brain className="w-3.5 h-3.5" />, category: '记忆', text: '信息要更新？', prompt: '我搬家了，新地址是 xx，帮我更新一下' },

  // 文档处理
  { icon: <FileText className="w-3.5 h-3.5" />, category: '文档', text: '简历想优化？', prompt: '看看我桌面的简历，帮我完善后保存成新版本' },
  { icon: <FileText className="w-3.5 h-3.5" />, category: '文档', text: 'PDF 不能编辑？', prompt: '把这份 PDF 合同转成 Word 方便修改' },
  { icon: <FileText className="w-3.5 h-3.5" />, category: '文档', text: '合同找关键条款？', prompt: '帮我找出这份合同里关于违约金的条款' },
  { icon: <FileText className="w-3.5 h-3.5" />, category: '文档', text: '文档要翻译？', prompt: '把这份中文文档翻译成英文，保存为新文件' },

  // 数据分析
  { icon: <BarChart3 className="w-3.5 h-3.5" />, category: '数据', text: 'Excel 数据分析？', prompt: '分析这份销售数据，找出增长最快的产品' },
  { icon: <BarChart3 className="w-3.5 h-3.5" />, category: '数据', text: '数据想做图表？', prompt: '把这份CSV数据做成柱状图和趋势线' },
  { icon: <BarChart3 className="w-3.5 h-3.5" />, category: '数据', text: '月光族？', prompt: '分析一下我这个月的消费记录，钱都花哪了' },

  // 截图分析
  { icon: <Camera className="w-3.5 h-3.5" />, category: '截图', text: '表格看不懂？', prompt: '截个屏，帮我分析这个 Excel 数据说明什么' },
  { icon: <Camera className="w-3.5 h-3.5" />, category: '截图', text: '报错看不懂？', prompt: '截个图，帮我看看这个错误怎么解决' },
  { icon: <Camera className="w-3.5 h-3.5" />, category: '截图', text: '设计想参考？', prompt: '截图这个网页，分析一下它的配色和布局' },

  // 办公写作
  { icon: <Mail className="w-3.5 h-3.5" />, category: '办公', text: '周报不会写？', prompt: '根据我这周做的事情，帮我写周报' },
  { icon: <Mail className="w-3.5 h-3.5" />, category: '办公', text: '邮件不会写？', prompt: '帮我写一封催款邮件，语气要客气但坚定' },
  { icon: <Briefcase className="w-3.5 h-3.5" />, category: '办公', text: '要跳槽了？', prompt: '帮我优化一下简历，我要投互联网公司' },
  { icon: <Briefcase className="w-3.5 h-3.5" />, category: '办公', text: '面试紧张？', prompt: '模拟一下产品经理的面试，问我几个问题' },
  { icon: <Briefcase className="w-3.5 h-3.5" />, category: '办公', text: '要涨薪了？', prompt: '帮我准备一下和老板谈涨薪的话术' },

  // 生活场景
  { icon: <Coffee className="w-3.5 h-3.5" />, category: '生活', text: '不知道吃啥？', prompt: '冰箱里有鸡蛋、番茄、豆腐，能做什么菜？' },
  { icon: <Gift className="w-3.5 h-3.5" />, category: '生活', text: '礼物选不好？', prompt: '送40岁的男领导什么生日礼物合适？' },
  { icon: <Globe className="w-3.5 h-3.5" />, category: '生活', text: '要出去旅游？', prompt: '帮我规划一个三天两夜的杭州旅行攻略' },
  { icon: <Calculator className="w-3.5 h-3.5" />, category: '生活', text: 'AA制算账？', prompt: '帮我算算这次聚餐每人应该出多少钱' },

  // 社交沟通
  { icon: <Heart className="w-3.5 h-3.5" />, category: '社交', text: '祝福词想不出？', prompt: '帮我写一段走心的生日祝福，送给闺蜜' },
  { icon: <MessageSquare className="w-3.5 h-3.5" />, category: '社交', text: '要道歉了？', prompt: '帮我写一段真诚的道歉信' },
  { icon: <MessageSquare className="w-3.5 h-3.5" />, category: '社交', text: '拒绝人太难？', prompt: '怎么委婉拒绝朋友借钱的请求' },

  // 学习成长
  { icon: <GraduationCap className="w-3.5 h-3.5" />, category: '学习', text: '概念看不懂？', prompt: '用大白话给我解释一下什么是区块链' },
  { icon: <GraduationCap className="w-3.5 h-3.5" />, category: '学习', text: '想学新技能？', prompt: '想学 Python，给我一个学习路线' },
  { icon: <GraduationCap className="w-3.5 h-3.5" />, category: '学习', text: '读书没时间？', prompt: '帮我总结一下《原则》这本书的核心观点' },

  // 自动化
  { icon: <Zap className="w-3.5 h-3.5" />, category: '自动化', text: '批量处理？', prompt: '把这100个图片都压缩到500KB以下' },
  { icon: <Clock className="w-3.5 h-3.5" />, category: '自动化', text: '定时提醒？', prompt: '每周五下午提醒我提交周报' },
  { icon: <Zap className="w-3.5 h-3.5" />, category: '自动化', text: '监控网页？', prompt: '这个商品降价了就提醒我' },

  // 创意设计
  { icon: <Palette className="w-3.5 h-3.5" />, category: '创意', text: '起名字头疼？', prompt: '帮我想几个有创意的公司名字，做宠物用品的' },
  { icon: <Palette className="w-3.5 h-3.5" />, category: '创意', text: '文案没灵感？', prompt: '帮我写一条卖口红的朋友圈文案' },
  { icon: <Sparkles className="w-3.5 h-3.5" />, category: '创意', text: '想发朋友圈？', prompt: '帮我写一段旅游发朋友圈的文字，要文艺点' },

  // 信息搜索
  { icon: <Search className="w-3.5 h-3.5" />, category: '搜索', text: '找之前聊过的？', prompt: '搜索一下我之前问过的关于 Excel 的问题' },
  { icon: <Search className="w-3.5 h-3.5" />, category: '搜索', text: '合同要审核？', prompt: '这份合同有没有坑？帮我看看' },

  // 新手引导
  { icon: <Lightbulb className="w-3.5 h-3.5" />, category: '探索', text: '第一次用？', prompt: '你能做什么？给我举几个例子' },
  { icon: <Lightbulb className="w-3.5 h-3.5" />, category: '探索', text: '功能太多？', prompt: '我是做财务的，你能帮我做什么？' },
];

// 随机打乱数组
const shuffleArray = <T,>(array: T[]): T[] => {
  const shuffled = [...array];
  for (let i = shuffled.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [shuffled[i], shuffled[j]] = [shuffled[j], shuffled[i]];
  }
  return shuffled;
};

interface ScrollingTipsProps {
  onSelectTip: (prompt: string) => void;
}

export function ScrollingTips({ onSelectTip }: ScrollingTipsProps) {
  const { t } = useTranslation('common');
  const [currentIndex, setCurrentIndex] = useState(0);
  const [isAnimating, setIsAnimating] = useState(false);
  const [shuffledTips] = useState(() => shuffleArray(tips));
  const [isPaused, setIsPaused] = useState(false);

  const nextTip = useCallback(() => {
    if (isPaused) return;
    setIsAnimating(true);
    setTimeout(() => {
      setCurrentIndex((prev) => (prev + 1) % shuffledTips.length);
      setIsAnimating(false);
    }, 300);
  }, [shuffledTips.length, isPaused]);

  useEffect(() => {
    const interval = setInterval(nextTip, 5000);
    return () => clearInterval(interval);
  }, [nextTip]);

  const currentTip = shuffledTips[currentIndex];

  const handleClick = () => {
    onSelectTip(currentTip.prompt);
  };

  return (
    <div
      className="w-full px-6 py-2"
      onMouseEnter={() => setIsPaused(true)}
      onMouseLeave={() => setIsPaused(false)}
    >
      <button
        onClick={handleClick}
        className={cn(
          "w-full flex items-center justify-center gap-2 py-2.5 px-4",
          "rounded-lg",
          "bg-gradient-to-r from-block-teal/10 via-block-teal/5 to-block-orange/10 dark:from-block-teal/5 dark:via-transparent dark:to-block-orange/5",
          "border border-block-teal/20 dark:border-white/5 hover:border-block-teal/30",
          "transition-all duration-300 ease-out",
          "hover:bg-block-teal/15 dark:hover:bg-block-teal/10",
          "group cursor-pointer",
          "shadow-sm hover:shadow-md dark:shadow-none"
        )}
      >
        <div className={cn(
          "flex items-center gap-2 transition-all duration-300",
          isAnimating ? "opacity-0 translate-y-2" : "opacity-100 translate-y-0"
        )}>
          {/* Icon */}
          <span className="text-block-teal opacity-80 group-hover:opacity-100 transition-opacity">
            {currentTip.icon}
          </span>

          {/* Category badge */}
          <span className="text-[10px] px-1.5 py-0.5 rounded bg-block-teal/10 dark:bg-white/5 text-block-teal dark:text-text-muted font-medium">
            {currentTip.category}
          </span>

          {/* Tip text */}
          <span className="text-sm text-text-default dark:text-text-muted group-hover:text-text-default transition-colors">
            {currentTip.text}
          </span>

          {/* Hint */}
          <span className="text-xs text-block-teal/60 group-hover:text-block-teal transition-colors ml-1">
            {t('tips.clickToTry')}
          </span>
        </div>
      </button>
    </div>
  );
}
