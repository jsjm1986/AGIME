import { useState, useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import {
  ChevronRight,
  ChevronDown,
  File,
  FileText,
  FileCode,
  Image,
  Folder,
  FolderOpen,
  Eye,
  Download,
} from 'lucide-react';
import { Button } from '../../ui/button';
import { SkillFile, formatPackageSize } from '../types';

interface FileTreeViewProps {
  files: SkillFile[];
  onViewFile?: (file: SkillFile) => void;
  onDownloadFile?: (file: SkillFile) => void;
  showActions?: boolean;
  compact?: boolean;
}

interface TreeNode {
  name: string;
  path: string;
  isDirectory: boolean;
  file?: SkillFile;
  children: Map<string, TreeNode>;
}

// Get file icon based on content type or extension
function getFileIcon(file: SkillFile) {
  const { contentType, path } = file;

  if (contentType.startsWith('image/')) {
    return <Image className="h-4 w-4 text-purple-500" />;
  }
  if (contentType.startsWith('text/markdown') || path.endsWith('.md')) {
    return <FileText className="h-4 w-4 text-blue-500" />;
  }
  if (
    contentType.includes('javascript') ||
    contentType.includes('typescript') ||
    contentType.includes('python') ||
    contentType.includes('json') ||
    path.match(/\.(js|ts|tsx|jsx|py|sh|bash|yaml|yml|toml|json|xml|html|css)$/)
  ) {
    return <FileCode className="h-4 w-4 text-green-500" />;
  }
  return <File className="h-4 w-4 text-text-muted" />;
}

// Get folder icon based on name
function getFolderIcon(name: string, isOpen: boolean) {
  const colors: Record<string, string> = {
    scripts: 'text-green-500',
    references: 'text-blue-500',
    assets: 'text-purple-500',
  };
  const color = colors[name] || 'text-yellow-500';

  return isOpen ? (
    <FolderOpen className={`h-4 w-4 ${color}`} />
  ) : (
    <Folder className={`h-4 w-4 ${color}`} />
  );
}

// Build tree structure from flat file list
function buildTree(files: SkillFile[]): TreeNode {
  const root: TreeNode = {
    name: '',
    path: '',
    isDirectory: true,
    children: new Map(),
  };

  for (const file of files) {
    const parts = file.path.split('/');
    let current = root;

    for (let i = 0; i < parts.length; i++) {
      const part = parts[i];
      const isLast = i === parts.length - 1;
      const currentPath = parts.slice(0, i + 1).join('/');

      if (!current.children.has(part)) {
        current.children.set(part, {
          name: part,
          path: currentPath,
          isDirectory: !isLast,
          file: isLast ? file : undefined,
          children: new Map(),
        });
      }
      current = current.children.get(part)!;
    }
  }

  return root;
}

// Sort children: directories first, then files, alphabetically
function sortChildren(children: Map<string, TreeNode>): TreeNode[] {
  return Array.from(children.values()).sort((a, b) => {
    if (a.isDirectory && !b.isDirectory) return -1;
    if (!a.isDirectory && b.isDirectory) return 1;
    return a.name.localeCompare(b.name);
  });
}

// Tree node component
function TreeNodeView({
  node,
  level,
  onViewFile,
  onDownloadFile,
  showActions,
  compact,
  expandedPaths,
  onToggle,
}: {
  node: TreeNode;
  level: number;
  onViewFile?: (file: SkillFile) => void;
  onDownloadFile?: (file: SkillFile) => void;
  showActions?: boolean;
  compact?: boolean;
  expandedPaths: Set<string>;
  onToggle: (path: string) => void;
}) {
  const { t } = useTranslation('team');
  const isExpanded = expandedPaths.has(node.path);
  const hasChildren = node.children.size > 0;

  if (node.isDirectory) {
    return (
      <div>
        <div
          className={`
            flex items-center gap-1 py-1 px-2 rounded cursor-pointer
            hover:bg-background-muted transition-colors
            ${compact ? 'text-xs' : 'text-sm'}
          `}
          style={{ paddingLeft: `${level * 16 + 8}px` }}
          onClick={() => onToggle(node.path)}
        >
          {hasChildren && (
            isExpanded ? (
              <ChevronDown className="h-3.5 w-3.5 text-text-muted shrink-0" />
            ) : (
              <ChevronRight className="h-3.5 w-3.5 text-text-muted shrink-0" />
            )
          )}
          {!hasChildren && <span className="w-3.5" />}
          {getFolderIcon(node.name, isExpanded)}
          <span className="font-medium text-text-default ml-1">{node.name}/</span>
        </div>
        {isExpanded && (
          <div>
            {sortChildren(node.children).map((child) => (
              <TreeNodeView
                key={child.path}
                node={child}
                level={level + 1}
                onViewFile={onViewFile}
                onDownloadFile={onDownloadFile}
                showActions={showActions}
                compact={compact}
                expandedPaths={expandedPaths}
                onToggle={onToggle}
              />
            ))}
          </div>
        )}
      </div>
    );
  }

  // File node
  return (
    <div
      className={`
        group flex items-center gap-1 py-1 px-2 rounded
        hover:bg-background-muted transition-colors
        ${compact ? 'text-xs' : 'text-sm'}
      `}
      style={{ paddingLeft: `${level * 16 + 8}px` }}
    >
      <span className="w-3.5" />
      {node.file && getFileIcon(node.file)}
      <span className="text-text-default ml-1 flex-1 truncate">{node.name}</span>
      {node.file && (
        <span className="text-text-muted text-xs ml-2 shrink-0">
          {formatPackageSize(node.file.size)}
        </span>
      )}
      {showActions && node.file && (
        <div className="opacity-0 group-hover:opacity-100 flex items-center gap-1 ml-2 shrink-0">
          {onViewFile && !node.file.isBinary && (
            <Button
              variant="ghost"
              size="sm"
              className="h-6 px-1.5"
              onClick={() => onViewFile(node.file!)}
              title={t('skillPackage.viewFile', '查看')}
            >
              <Eye className="h-3.5 w-3.5" />
            </Button>
          )}
          {onDownloadFile && (
            <Button
              variant="ghost"
              size="sm"
              className="h-6 px-1.5"
              onClick={() => onDownloadFile(node.file!)}
              title={t('skillPackage.downloadFile', '下载')}
            >
              <Download className="h-3.5 w-3.5" />
            </Button>
          )}
        </div>
      )}
    </div>
  );
}

export function FileTreeView({
  files,
  onViewFile,
  onDownloadFile,
  showActions = true,
  compact = false,
}: FileTreeViewProps) {
  const { t } = useTranslation('team');

  // Build tree structure
  const tree = useMemo(() => buildTree(files), [files]);

  // Default expanded paths (root level directories)
  const [expandedPaths, setExpandedPaths] = useState<Set<string>>(() => {
    const initial = new Set<string>();
    // Expand root level directories by default
    for (const child of tree.children.values()) {
      if (child.isDirectory) {
        initial.add(child.path);
      }
    }
    return initial;
  });

  const handleToggle = (path: string) => {
    setExpandedPaths((prev) => {
      const next = new Set(prev);
      if (next.has(path)) {
        next.delete(path);
      } else {
        next.add(path);
      }
      return next;
    });
  };

  if (files.length === 0) {
    return (
      <div className="text-center py-4 text-text-muted text-sm">
        {t('skillPackage.noFiles', '没有附加文件')}
      </div>
    );
  }

  return (
    <div className="border rounded-lg overflow-hidden">
      <div className="bg-background-muted px-3 py-2 border-b">
        <p className={`font-medium text-text-default ${compact ? 'text-xs' : 'text-sm'}`}>
          {t('skillPackage.fileStructure', '文件结构')}
          <span className="text-text-muted font-normal ml-2">
            ({files.length} {t('skillPackage.filesCount', '个文件')})
          </span>
        </p>
      </div>
      <div className="py-1 max-h-[300px] overflow-y-auto">
        {sortChildren(tree.children).map((child) => (
          <TreeNodeView
            key={child.path}
            node={child}
            level={0}
            onViewFile={onViewFile}
            onDownloadFile={onDownloadFile}
            showActions={showActions}
            compact={compact}
            expandedPaths={expandedPaths}
            onToggle={handleToggle}
          />
        ))}
      </div>
    </div>
  );
}

export default FileTreeView;
