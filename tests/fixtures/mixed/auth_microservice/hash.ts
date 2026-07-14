export interface HashResult {
	hash: string;
	algorithm: string;
	rounds: number;
}

export function format_hash_result(rawHash: string, algorithm: string): HashResult {
	return {
		hash: rawHash,
		algorithm,
		rounds: 12,
	};
}
