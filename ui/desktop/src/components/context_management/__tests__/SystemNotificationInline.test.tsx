import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { SystemNotificationInline } from '../SystemNotificationInline';
import { Message } from '../../../api';

function createInlineMessage(msg: string): Message {
  return {
    id: 'msg_cfpm_test',
    role: 'assistant',
    created: Date.now(),
    metadata: {
      userVisible: true,
      agentVisible: true,
    },
    content: [
      {
        type: 'systemNotification',
        notificationType: 'inlineMessage',
        msg,
      },
    ],
  };
}

describe('SystemNotificationInline', () => {
  it('renders plain inline notification when message is not CFPM payload', () => {
    render(<SystemNotificationInline message={createInlineMessage('Context compacted.')} />);

    expect(screen.getByText('Context compacted.')).toBeInTheDocument();
  });

  it('renders CFPM localized summary for structured payload', () => {
    const payload =
      '[CFPM_RUNTIME_V1] {"version":"v1","verbosity":"brief","reason":"turn_checkpoint","mode":"merge","acceptedCount":2,"rejectedCount":1,"prunedCount":0,"factCount":9,"rejectedReasonBreakdown":["artifact_unhelpful=1"]}';
    render(<SystemNotificationInline message={createInlineMessage(payload)} />);

    expect(screen.getByText(/systemNotification\.cfpmSummary/)).toBeInTheDocument();
    expect(screen.queryByText(/\[CFPM_RUNTIME_V1\]/)).not.toBeInTheDocument();
    expect(screen.queryByRole('button')).not.toBeInTheDocument();
  });

  it('shows debug details section when toggled', async () => {
    const user = userEvent.setup();
    const payload =
      '[CFPM_RUNTIME_V1] {"version":"v1","verbosity":"debug","reason":"turn_checkpoint","mode":"merge","acceptedCount":2,"rejectedCount":1,"prunedCount":1,"factCount":9,"rejectedReasonBreakdown":["artifact_unhelpful=1","duplicate=1"]}';
    render(<SystemNotificationInline message={createInlineMessage(payload)} />);

    const detailsButton = screen.getByRole('button');
    await user.click(detailsButton);

    expect(screen.getByText(/artifact_unhelpful=1/)).toBeInTheDocument();
    expect(screen.getByText(/duplicate=1/)).toBeInTheDocument();
  });

  it('renders CFPM tool gate localized summary for structured payload', () => {
    const payload =
      '[CFPM_TOOL_GATE_V1] {"version":"v1","verbosity":"brief","action":"rewrite_known_folder_probe","tool":"developer__shell_command","target":"desktop","path":"C:\\\\Users\\\\jsjm\\\\OneDrive\\\\Desktop","originalCommand":"Get-ChildItem \\"$env:USERPROFILE/Desktop\\"","rewrittenCommand":"Get-ChildItem \\"C:\\\\Users\\\\jsjm\\\\OneDrive\\\\Desktop\\""}';
    render(<SystemNotificationInline message={createInlineMessage(payload)} />);

    expect(screen.getByText(/systemNotification\.cfpmToolGateSummary/)).toBeInTheDocument();
    expect(screen.queryByRole('button')).not.toBeInTheDocument();
  });

  it('shows CFPM tool gate debug command details when toggled', async () => {
    const user = userEvent.setup();
    const payload =
      '[CFPM_TOOL_GATE_V1] {"version":"v1","verbosity":"debug","action":"rewrite_known_folder_probe","tool":"developer__shell_command","target":"desktop","path":"C:\\\\Users\\\\jsjm\\\\OneDrive\\\\Desktop","originalCommand":"Get-ChildItem \\"$env:USERPROFILE/Desktop\\"","rewrittenCommand":"Get-ChildItem \\"C:\\\\Users\\\\jsjm\\\\OneDrive\\\\Desktop\\""}';
    render(<SystemNotificationInline message={createInlineMessage(payload)} />);

    const detailsButton = screen.getByRole('button');
    await user.click(detailsButton);

    expect(
      screen.getByText(/Get-ChildItem "\$env:USERPROFILE\/Desktop"/)
    ).toBeInTheDocument();
    expect(screen.getByText(/Get-ChildItem "C:\\Users\\jsjm\\OneDrive\\Desktop"/)).toBeInTheDocument();
  });
});
