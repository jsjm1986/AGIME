import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { GroupedExtensionLoadingToast } from '../GroupedExtensionLoadingToast';

const renderWithRouter = (component: React.ReactElement) => {
  return render(<MemoryRouter>{component}</MemoryRouter>);
};

describe('GroupedExtensionLoadingToast', () => {
  it('renders loading state correctly', () => {
    const extensions = [
      { name: 'developer', status: 'loading' as const },
      { name: 'memory', status: 'loading' as const },
    ];

    renderWithRouter(
      <GroupedExtensionLoadingToast extensions={extensions} totalCount={2} isComplete={false} />
    );

    expect(screen.getByText('Loading 2 extension(s)...')).toBeInTheDocument();
    expect(screen.getByText(/2\s+loading\.\.\./)).toBeInTheDocument();
  });

  it('renders success state correctly', () => {
    const extensions = [
      { name: 'developer', status: 'success' as const },
      { name: 'memory', status: 'success' as const },
    ];

    renderWithRouter(
      <GroupedExtensionLoadingToast extensions={extensions} totalCount={2} isComplete={true} />
    );

    expect(screen.getByText('Successfully loaded 2 extension(s)')).toBeInTheDocument();
  });

  it('renders partial failure state correctly', () => {
    const extensions = [
      { name: 'developer', status: 'success' as const },
      { name: 'memory', status: 'error' as const, error: 'Failed to connect' },
    ];

    renderWithRouter(
      <GroupedExtensionLoadingToast extensions={extensions} totalCount={2} isComplete={true} />
    );

    expect(screen.getByText('Loaded 1/2 extension(s)')).toBeInTheDocument();
    expect(screen.getByText('1 extension(s) failed to load')).toBeInTheDocument();
  });

  it('renders single extension correctly', () => {
    const extensions = [{ name: 'developer', status: 'success' as const }];

    renderWithRouter(
      <GroupedExtensionLoadingToast extensions={extensions} totalCount={1} isComplete={true} />
    );

    expect(screen.getByText('Successfully loaded 1 extension(s)')).toBeInTheDocument();
  });

  it('renders mixed status states correctly', () => {
    const extensions = [
      { name: 'developer', status: 'success' as const },
      { name: 'memory', status: 'loading' as const },
      { name: 'Square MCP Server', status: 'error' as const, error: 'Connection failed' },
    ];

    renderWithRouter(
      <GroupedExtensionLoadingToast extensions={extensions} totalCount={3} isComplete={false} />
    );

    // Summary should show loading state with error count
    expect(screen.getByText('Loading 3 extension(s)...')).toBeInTheDocument();
    expect(screen.getByText(/1\s+loading\.\.\./)).toBeInTheDocument();
  });
});
