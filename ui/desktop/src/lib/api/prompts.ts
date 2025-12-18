import { client } from '../../api/client.gen';

export interface PromptTemplate {
  name: string;
  display_name: string;
  description: string;
  content: string;
}

export interface PromptResponse {
  content: string;
  is_enabled: boolean;
  is_custom: boolean;
}

export interface DefaultPromptsResponse {
  prompts: PromptTemplate[];
}

export interface SetSystemPromptRequest {
  content: string;
  enabled: boolean;
}

export const getSystemPrompt = () => {
  return client.get<PromptResponse>({
    url: '/config/prompts/system',
  });
};

export const setSystemPrompt = (options: { body: SetSystemPromptRequest }) => {
  return client.post<string>({
    url: '/config/prompts/system',
    body: options.body,
    headers: {
      'Content-Type': 'application/json',
    },
  });
};

export const resetSystemPrompt = () => {
  return client.post<string>({
    url: '/config/prompts/system/reset',
  });
};

export const getDefaultPrompts = () => {
  return client.get<DefaultPromptsResponse>({
    url: '/config/prompts/defaults',
  });
};
