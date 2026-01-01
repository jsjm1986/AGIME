/**
 * Quick Setup API Service
 * Handles all API calls for the quick setup wizard
 */

import {
  detectProvider,
  createCustomProvider,
  getProviderModels,
  setConfigProvider,
  upsertConfig,
} from '../../../../api/sdk.gen';
import type { CredentialsData } from '../steps/CredentialsForm';
import type { ModelCapabilities } from '../steps/CapabilityConfirm';
import type { ProviderPreset } from '../data/providerPresets';

export interface QuickSetupResult {
  success: boolean;
  message?: string;
  error?: string;
  detectedProvider?: string;
  detectedModels?: string[];
  createdProviderId?: string;  // ID of newly created custom provider
}

export interface ModelsResult {
  success: boolean;
  models: string[];
  error?: string;
}

/**
 * Validate credentials by detecting the provider from the API key
 * For proxy/china providers, we skip detection and just save the config
 */
export async function validateCredentials(
  provider: ProviderPreset,
  credentials: CredentialsData
): Promise<QuickSetupResult> {
  try {
    // For proxy providers, china providers, or custom base URLs,
    // we can't use detectProvider - just save the config
    const isProxyOrChina = provider.category === 'proxy' || provider.category === 'china';
    const hasCustomUrl = credentials.useCustomUrl || provider.id === 'custom';

    if (isProxyOrChina || hasCustomUrl) {
      // Save the API key to config
      const envKey = provider.apiKeyEnv || 'OPENAI_API_KEY';

      await upsertConfig({
        body: {
          key: envKey,
          value: credentials.apiKey,
          is_secret: true,
        },
      });

      // Save base URL
      if (credentials.baseUrl) {
        // Determine the base URL env key based on provider
        let baseUrlKey = envKey.replace('_API_KEY', '_BASE_URL').replace('_KEY', '_BASE_URL');
        // For OpenAI compatible, use OPENAI_BASE_URL
        if (provider.engine === 'openai' && !baseUrlKey.includes('BASE_URL')) {
          baseUrlKey = 'OPENAI_BASE_URL';
        }

        await upsertConfig({
          body: {
            key: baseUrlKey,
            value: credentials.baseUrl,
            is_secret: false,
          },
        });
      }

      return {
        success: true,
        message: `配置已保存 - ${provider.displayName}`,
      };
    }

    // For standard international providers (OpenAI, Anthropic, Google directly),
    // use detectProvider to validate the API key
    const result = await detectProvider({
      body: {
        api_key: credentials.apiKey,
      },
    });

    if (result.data) {
      // Save the API key to config
      const envKey = provider.apiKeyEnv;
      await upsertConfig({
        body: {
          key: envKey,
          value: credentials.apiKey,
          is_secret: true,
        },
      });

      return {
        success: true,
        message: `连接验证成功！检测到 ${result.data.provider_name}`,
        detectedProvider: result.data.provider_name,
        detectedModels: result.data.models,
      };
    }

    return {
      success: false,
      error: '无法验证 API Key，请检查是否正确',
    };
  } catch (error) {
    // Check if it's a 404 (no provider found)
    if (error && typeof error === 'object' && 'status' in error && error.status === 404) {
      return {
        success: false,
        error: '无法识别此 API Key 对应的服务商，请检查 Key 是否正确',
      };
    }

    return {
      success: false,
      error: error instanceof Error ? error.message : '验证失败，请检查网络连接和 API Key',
    };
  }
}

/**
 * Fetch available models from the provider
 */
export async function fetchProviderModels(
  providerName: string
): Promise<ModelsResult> {
  try {
    const result = await getProviderModels({
      path: {
        name: providerName,
      },
    });

    if (result.data && Array.isArray(result.data)) {
      return {
        success: true,
        models: result.data as string[],
      };
    }

    return {
      success: false,
      models: [],
      error: '无法获取模型列表',
    };
  } catch (error) {
    return {
      success: false,
      models: [],
      error: error instanceof Error ? error.message : '获取模型列表失败',
    };
  }
}

/**
 * Create or update a custom provider with the given configuration
 * Returns the created provider ID from the backend
 */
export async function saveCustomProvider(
  displayName: string,
  credentials: CredentialsData,
  modelName: string,
  capabilities: ModelCapabilities
): Promise<QuickSetupResult> {
  try {
    const result = await createCustomProvider({
      body: {
        api_key: credentials.apiKey,
        api_url: credentials.baseUrl,
        display_name: displayName,
        engine: credentials.engine,
        models: [modelName],
        supports_streaming: capabilities.supportsStreaming,
        headers: null,
      },
    });

    if (result.error) {
      return {
        success: false,
        error: '创建配置失败',
      };
    }

    // Backend returns "Custom provider added - ID: {id}"
    // Parse the ID from the response
    let createdId: string | undefined;
    if (result.data && typeof result.data === 'string') {
      const match = result.data.match(/ID:\s*(.+)$/);
      if (match) {
        createdId = match[1].trim();
      }
    }

    return {
      success: true,
      message: '配置创建成功！',
      createdProviderId: createdId,
    };
  } catch (error) {
    return {
      success: false,
      error: error instanceof Error ? error.message : '保存配置失败',
    };
  }
}

/**
 * Set the active provider and model
 */
export async function setActiveProvider(
  providerName: string,
  modelName: string
): Promise<QuickSetupResult> {
  try {
    await setConfigProvider({
      body: {
        provider: providerName,
        model: modelName,
      },
    });

    return {
      success: true,
      message: '已切换到新配置！',
    };
  } catch (error) {
    return {
      success: false,
      error: error instanceof Error ? error.message : '设置提供商失败',
    };
  }
}

/**
 * Complete the quick setup process
 * This combines creating/updating the provider and setting it as active
 */
export async function completeQuickSetup(
  provider: ProviderPreset,
  credentials: CredentialsData,
  modelName: string,
  capabilities: ModelCapabilities
): Promise<QuickSetupResult> {
  try {
    // Determine if we need to create a custom provider
    // - proxy category always needs custom provider
    // - providers with name starting with "custom_" need custom provider
    // - providers with custom base URL need custom provider
    const isProxyCategory = provider.category === 'proxy';
    const isNamedCustom = provider.name.startsWith('custom_');
    const hasNonDefaultBaseUrl = credentials.useCustomUrl || provider.id === 'custom';
    const needsCustomProvider = isProxyCategory || isNamedCustom || hasNonDefaultBaseUrl;

    if (!needsCustomProvider) {
      // For official providers, use the provider name (not id) and set the config
      const setResult = await setActiveProvider(provider.name, modelName);
      if (!setResult.success) {
        return setResult;
      }

      return {
        success: true,
        message: '配置完成！已切换到 ' + provider.displayName,
      };
    }

    // For providers needing custom setup, create a custom provider first
    const saveResult = await saveCustomProvider(
      provider.displayName,
      credentials,
      modelName,
      capabilities
    );

    if (!saveResult.success) {
      return saveResult;
    }

    // Use the provider ID returned by the backend (correctly handles Chinese characters in displayName)
    const customProviderId = saveResult.createdProviderId;
    if (!customProviderId) {
      return {
        success: false,
        error: '创建配置成功，但无法获取提供商 ID',
      };
    }

    const setResult = await setActiveProvider(customProviderId, modelName);

    if (!setResult.success) {
      return {
        success: false,
        error: '配置已保存，但切换失败: ' + setResult.error,
      };
    }

    return {
      success: true,
      message: '配置完成！已切换到 ' + provider.displayName,
    };
  } catch (error) {
    return {
      success: false,
      error: error instanceof Error ? error.message : '配置过程中出错',
    };
  }
}

/**
 * Validate a model by checking if it's available from the provider
 */
export async function validateModel(
  providerName: string,
  _credentials: CredentialsData,
  modelName: string
): Promise<QuickSetupResult> {
  try {
    // Try to get available models from the provider
    const modelsResult = await fetchProviderModels(providerName);

    if (modelsResult.success && modelsResult.models.length > 0) {
      // Check if the selected model is in the available list
      const modelExists = modelsResult.models.some(
        (m) => m.toLowerCase() === modelName.toLowerCase() || m.includes(modelName) || modelName.includes(m)
      );

      if (modelExists) {
        return {
          success: true,
          message: '模型验证成功！',
        };
      } else {
        // Model not in list, but might still be valid (some providers don't list all models)
        return {
          success: true,
          message: '模型名称已确认（未在可用列表中找到，但可能仍然有效）',
        };
      }
    }

    // Couldn't get model list, assume valid
    return {
      success: true,
      message: '模型名称格式正确',
    };
  } catch (error) {
    // On error, still allow proceeding
    return {
      success: true,
      message: '无法验证模型列表，将在使用时验证',
    };
  }
}

/**
 * Probe model capabilities by making test API calls
 * This attempts to determine what features the model supports
 */
export async function probeModelCapabilities(
  _providerName: string,
  _credentials: CredentialsData,
  _modelName: string
): Promise<Partial<ModelCapabilities>> {
  // This is a placeholder for capability probing
  // In a full implementation, this would:
  // 1. Try sending a test message with tools to check tool support
  // 2. Try sending an image to check vision support
  // 3. Check streaming by observing the response format

  return {
    supportsTools: true,
    supportsStreaming: true,
    supportsVision: false, // Conservative default
  };
}
