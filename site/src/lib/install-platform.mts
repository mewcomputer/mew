export function defaultInstallPlatform(platform: string, userAgent: string): 'macos' | 'linux' {
    return /^Linux\b/.test(platform) && !/\b(Android|CrOS)\b/i.test(userAgent) ? 'linux' : 'macos';
}
