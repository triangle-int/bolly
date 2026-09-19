/**
 * Save the credential in its provider's slot before activating that provider.
 * @param {'anthropic' | 'openai'} provider
 * @param {string} key
 * @param {{ updateLlmConfig: (payload: {api_key?: string, openai?: string}) => Promise<void>, updateProvider: (provider: 'anthropic' | 'openai') => Promise<unknown> }} api
 */
export async function saveOnboardingProvider(provider, key, api) {
	await api.updateLlmConfig(provider === 'openai' ? { openai: key } : { api_key: key });
	await api.updateProvider(provider);
}
