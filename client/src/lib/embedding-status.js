/**
 * @param {{status: string, provider?: string, model?: string, reason?: string | null, needs_restart?: boolean} | undefined} embedding
 */
export function embeddingStatusText(embedding) {
    if (!embedding) return 'Semantic memory status unavailable.';
    const backend = embedding.provider === 'openai_compatible' ? 'Local OpenAI-compatible (unauthenticated)'
        : embedding.provider === 'openai' ? 'OpenAI' : 'Text embeddings';
    const model = embedding.model ? ` (${embedding.model})` : '';
    const health = embedding.status === 'available' ? 'available'
        : embedding.status === 'unverified' ? 'configured, not yet verified'
        : `unavailable${embedding.reason ? `: ${embedding.reason}` : ''}`;
    const restart = embedding.needs_restart ? ' Restart the server to apply saved embedding settings or the OpenAI key.' : '';
    return `${backend} semantic memory${model}: ${health}. Keyword search (BM25) remains available.${restart}`;
}
