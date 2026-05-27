import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';
import { SystemNotificationInline } from '../SystemNotificationInline';
import { Message } from '../../../api';

function createInlineMessage(msg: string): Message {
  return {
    id: 'msg_inline_test',
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
  it('renders plain inline notification message', () => {
    render(<SystemNotificationInline message={createInlineMessage('Context compacted.')} />);

    expect(screen.getByText('Context compacted.')).toBeInTheDocument();
  });
});
