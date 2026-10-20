<script lang="ts">
    import { onMount } from 'svelte';
    import { t } from '$lib/i18n';
    import { getSessionCookieClientSide } from '$lib/auth';
    import { api } from '$lib/playback/api';
    interface Config {
        min_resume_percentage: number; max_resume_percentage: number; min_resume_duration_seconds: number;
        streaming_bitrate_limit_mbps: number | null; local_networks: string[]; trusted_proxies: string[];
        ffmpeg_path: string; transcoding_temp_path: string; hardware_acceleration: string; hardware_encoding: boolean;
        hardware_decoding_codecs: string[]; decode_hevc_10bit: boolean; decode_vp9_10bit: boolean; enhanced_nvdec: boolean;
        encoding_threads: number; encoder_preset: string; h264_crf: number; max_output_height: number;
        video_bitrate_kbps: number; audio_bitrate_kbps: number; audio_vbr: boolean; downmix_audio_boost: number;
        max_muxing_queue_size: number; deinterlace_method: string; deinterlace_double_rate: boolean;
        tonemapping: boolean; tonemapping_algorithm: string; tonemapping_desat: number; tonemapping_peak: number; tonemapping_param: number | null;
        tonemapping_backend: string; hardware_device_index: number; hevc_encoding: boolean; allow_remux: boolean;
        hls_segment_seconds: number; throttling: boolean; throttle_lead_seconds: number; segment_retention_seconds: number;
        max_concurrent_transcodes: number; downmix_algorithm: string; subtitle_fallback_charset: string;
    }
    interface Caps {
        ffmpeg_path: string; ffprobe_path: string; version: string; nvenc_available: boolean; software_tonemapping: boolean;
        jellyfin_build: boolean; vulkan_available: boolean; tonemapping_backends: string[];
    }
    let config = $state<Config>();
    let networks = $state(''), proxies = $state('');
    let remoteLimit = $state<number | undefined>(), toneParam = $state<number | undefined>();
    let caps = $state<Caps>();
    let busy = $state(false), checking = $state(false), failed = $state(false), saved = $state(false), checkFailed = $state(false);
    let token = '';
    onMount(async () => {
        token = getSessionCookieClientSide() ?? '';
        try {
            config = await api<Config>(token, '/api/admin/playback/settings');
            networks = config.local_networks.join('\n'); proxies = config.trusted_proxies.join('\n');
            remoteLimit = config.streaming_bitrate_limit_mbps ?? undefined; toneParam = config.tonemapping_param ?? undefined;
        } catch { failed = true; }
    });
    function payload() {
        return { ...config, streaming_bitrate_limit_mbps: remoteLimit ?? null, tonemapping_param: toneParam ?? null,
            local_networks: networks.split(/[,\n]/).map(s => s.trim()).filter(Boolean),
            trusted_proxies: proxies.split(/[,\n]/).map(s => s.trim()).filter(Boolean) };
    }
    async function save(event: SubmitEvent) {
        event.preventDefault(); busy = true; saved = false; failed = false;
        try { await api(token, '/api/admin/playback/settings', 'PUT', payload()); saved = true; }
        catch { failed = true; } finally { busy = false; }
    }
    async function check() {
        checking = true; caps = undefined; checkFailed = false;
        try { caps = await api<Caps>(token, '/api/admin/playback/capabilities', 'POST', payload()); }
        catch { checkFailed = true; } finally { checking = false; }
    }
</script>
<section class="mb-12">
    <h1 class="mb-6 text-2xl">{t('playback_settings.title')}</h1>
    {#if config}
    <form onsubmit={save} class="flex flex-col gap-6">
        <fieldset>
            <legend>{t('playback_settings.resume')}</legend>
            <label>{t('playback_settings.minimum_resume_percentage')}<input type="number" min="0" max="99" step="0.1" required bind:value={config.min_resume_percentage} /></label>
            <p>{t('playback_settings.resume_min_hint')}</p>
            <label>{t('playback_settings.maximum_resume_percentage')}<input type="number" min="1" max="100" step="0.1" required bind:value={config.max_resume_percentage} /></label>
            <p>{t('playback_settings.resume_max_hint')}</p>
            <label>{t('playback_settings.minimum_video_duration_for_resume_seconds')}<input type="number" min="0" max="604800" required bind:value={config.min_resume_duration_seconds} /></label>
            <p>{t('playback_settings.resume_duration_hint')}</p>
        </fieldset>
        <fieldset>
            <legend>{t('playback_settings.streaming')}</legend>
            <label>{t('playback_settings.remote_streaming_bitrate_limit_mbps')}<input type="number" min="0" max="10000" step="0.25" bind:value={remoteLimit} /></label>
            <p>{t('playback_settings.remote_limit_hint')}</p>
            <details><summary>{t('playback_settings.home_network_and_proxies')}</summary>
                <label>{t('playback_settings.local_networks_cidr_one_per_line')}<textarea rows="5" bind:value={networks}></textarea></label>
                <label>{t('playback_settings.trusted_proxies_cidr')}<textarea rows="3" bind:value={proxies}></textarea></label>
                <p>{t('playback_settings.trusted_proxies_hint')}</p>
            </details>
        </fieldset>
        <fieldset>
            <legend>{t('playback_settings.transcoding')}</legend>
            <label>{t('playback_settings.ffmpeg_executable_or_folder')}<input bind:value={config.ffmpeg_path} placeholder={t('playback_settings.server_environment_path')} /></label>
            <label>{t('playback_settings.temporary_files_folder')}<input bind:value={config.transcoding_temp_path} placeholder={t('playback_settings.default_server_folder')} /></label>
            <button type="button" class="secondary" disabled={checking} onclick={check}>{checking ? t('playback_settings.checking') : t('playback_settings.check_ffmpeg_and_nvidia_gpu')}</button>
            {#if caps}<div role="status" class="break-all rounded bg-white/5 p-3 text-sm">{caps.version}<br />{t('playback_settings.ffmpeg_label')} {caps.ffmpeg_path}<br />{t('playback_settings.ffprobe_label')} {caps.ffprobe_path}<br />{t('playback_settings.nvenc_label')} {caps.nvenc_available ? t('playback_settings.encoding_test_passed') : t('playback_settings.unavailable')}<br />{t('playback_settings.hdr_sdr_label')} {caps.tonemapping_backends.length ? caps.tonemapping_backends.join(', ') : t('playback_settings.filters_unavailable')}<br />{t('playback_settings.vulkan_label')} {caps.vulkan_available ? t('playback_settings.available') : t('playback_settings.unavailable')}{#if caps.jellyfin_build}<br />{t('playback_settings.jellyfin_build')}{/if}</div>{/if}
            {#if checkFailed}<p role="alert">{t('playback_settings.ffmpeg_check_failed')}</p>{/if}
            <label>{t('playback_settings.hardware_acceleration')}<select bind:value={config.hardware_acceleration}><option value="none">{t('playback_settings.none_cpu')}</option><option value="nvenc">{t('playback_settings.nvidia_nvenc_nvdec')}</option></select></label>
            {#if config.hardware_acceleration === 'nvenc'}
                <label class="check"><input type="checkbox" bind:checked={config.hardware_encoding} />{t('playback_settings.hardware_encoding')}</label>
                <p>{t('playback_settings.hardware_decoding_for')}</p>
                <div class="flex flex-wrap gap-4">{#each ['h264','hevc','mpeg2video','mpeg4','vc1','vp8','vp9','av1'] as codec (codec)}<label class="check"><input type="checkbox" value={codec} bind:group={config.hardware_decoding_codecs} />{codec.toUpperCase()}</label>{/each}</div>
                <label class="check"><input type="checkbox" bind:checked={config.decode_hevc_10bit} />{t('playback_settings.hevc_10bit')}</label>
                <label class="check"><input type="checkbox" bind:checked={config.decode_vp9_10bit} />{t('playback_settings.vp9_10bit')}</label>
                <label class="check"><input type="checkbox" bind:checked={config.enhanced_nvdec} />{t('playback_settings.enhanced_nvdec_disable_to_use_cuvid')}</label>
                <label>{t('playback_settings.gpu_index')}<input type="number" min="0" max="15" required bind:value={config.hardware_device_index} /></label>
                <p>{t('playback_settings.gpu_index_hint')}</p>
                <label class="check"><input type="checkbox" bind:checked={config.hevc_encoding} />{t('playback_settings.hevc_encoding')}</label>
            {/if}
            <label>{t('playback_settings.maximum_output_height')}<select bind:value={config.max_output_height}>{#each [360,480,720,1080,1440,2160,4320] as height (height)}<option value={height}>{t('playback_settings.height_option', { height })}</option>{/each}</select></label>
            <label>{t('playback_settings.maximum_video_bitrate_kbps')}<input type="number" min="100" max="200000" required bind:value={config.video_bitrate_kbps} /></label>
            <label>{t('playback_settings.thread_count_0_automatic')}<input type="number" min="0" max="256" required bind:value={config.encoding_threads} /></label>
            <label>{t('playback_settings.encoder_preset')}<select bind:value={config.encoder_preset}>{#each ['ultrafast','superfast','veryfast','faster','fast','medium','slow','slower','veryslow'] as preset (preset)}<option value={preset}>{t(`playback_settings.presets.${preset}`)}</option>{/each}</select></label>
            <p>{t('playback_settings.encoder_preset_hint')}</p>
            <label>{t('playback_settings.h264_crf')}<input type="number" min="0" max="51" required bind:value={config.h264_crf} /></label>
            <p>{t('playback_settings.crf_hint')}</p>
            <label>{t('playback_settings.audio_bitrate_kbps')}<input type="number" min="32" max="512" required bind:value={config.audio_bitrate_kbps} /></label>
            <label class="check"><input type="checkbox" bind:checked={config.audio_vbr} />{t('playback_settings.vbr_audio_remote_playback_uses_the_bitrate_limit')}</label>
            <label>{t('playback_settings.stereo_downmix_boost')}<input type="number" min="0.5" max="3" step="0.1" required bind:value={config.downmix_audio_boost} /></label>
            <label>{t('playback_settings.maximum_muxing_queue_size')}<input type="number" min="128" max="65536" required bind:value={config.max_muxing_queue_size} /></label>
            <label>{t('playback_settings.deinterlacing')}<select bind:value={config.deinterlace_method}><option value="none">{t('playback_settings.disabled')}</option><option value="yadif">{t('playback_settings.yadif')}</option><option value="bwdif">{t('playback_settings.bwdif')}</option></select></label>
            <label class="check"><input type="checkbox" bind:checked={config.deinterlace_double_rate} />{t('playback_settings.double_the_frame_rate_when_deinterlacing')}</label>
            <label class="check"><input type="checkbox" bind:checked={config.tonemapping} />{t('playback_settings.hdr10_hlg_sdr_tone_mapping')}</label>
            {#if config.tonemapping}
                <p>{t('playback_settings.tonemapping_hint')}</p>
                <label>{t('playback_settings.tonemapping_backend')}<select bind:value={config.tonemapping_backend}>{#each ['auto','cuda','libplacebo','opencl','software'] as backend (backend)}<option value={backend}>{t(`playback_settings.backends.${backend}`)}</option>{/each}</select></label>
                <label>{t('playback_settings.algorithm')}<select bind:value={config.tonemapping_algorithm}>{#each ['bt2390','hable','reinhard','mobius','clip','linear','gamma'] as algorithm (algorithm)}<option value={algorithm}>{t(`playback_settings.algorithms.${algorithm}`)}</option>{/each}</select></label>
                <label>{t('playback_settings.desaturation')}<input type="number" min="0" max="100" step="0.00001" required bind:value={config.tonemapping_desat} /></label>
                <label>{t('playback_settings.signal_peak_0_automatic')}<input type="number" min="0" max="10000" step="0.00001" required bind:value={config.tonemapping_peak} /></label>
                <label>{t('playback_settings.algorithm_parameter_optional')}<input type="number" min="0" max="100" step="0.00001" bind:value={toneParam} /></label>
            {/if}
        </fieldset>
        <fieldset>
            <legend>{t('playback_settings.hls')}</legend>
            <label class="check"><input type="checkbox" bind:checked={config.allow_remux} />{t('playback_settings.allow_remux')}</label>
            <p>{t('playback_settings.allow_remux_hint')}</p>
            <label>{t('playback_settings.segment_seconds')}<input type="number" min="2" max="10" required bind:value={config.hls_segment_seconds} /></label>
            <label class="check"><input type="checkbox" bind:checked={config.throttling} />{t('playback_settings.throttling')}</label>
            <label>{t('playback_settings.throttle_lead_seconds')}<input type="number" min="10" max="600" required bind:value={config.throttle_lead_seconds} /></label>
            <p>{t('playback_settings.throttling_hint')}</p>
            <label>{t('playback_settings.segment_retention_seconds')}<input type="number" min="10" max="3600" required bind:value={config.segment_retention_seconds} /></label>
            <label>{t('playback_settings.max_concurrent_transcodes')}<input type="number" min="1" max="32" required bind:value={config.max_concurrent_transcodes} /></label>
            <label>{t('playback_settings.downmix_algorithm')}<select bind:value={config.downmix_algorithm}>{#each ['standard','dialogue','night','pro_logic_ii'] as mode (mode)}<option value={mode}>{t(`player.downmix_modes.${mode}`)}</option>{/each}</select></label>
            <p>{t('playback_settings.downmix_hint')}</p>
            <label>{t('playback_settings.subtitle_charset')}<input required maxlength="32" pattern="[A-Za-z0-9_\-]+" bind:value={config.subtitle_fallback_charset} /></label>
            <p>{t('playback_settings.subtitle_charset_hint')}</p>
        </fieldset>
        <button disabled={busy} class="rounded bg-gold px-4 py-3 text-black disabled:opacity-50">{busy ? t('playback_settings.saving') : t('playback_settings.save_playback_settings')}</button>
    </form>
    {:else if !failed}<p role="status">{t('playback_settings.loading')}</p>{/if}
    {#if saved}<p role="status" class="mt-4 text-gold">{t('playback_settings.saved')}</p>{/if}
    {#if failed}<p role="alert" class="mt-4">{t('playback_settings.load_or_save_failed')}</p>{/if}
</section>
<style>
    fieldset { display:flex; flex-direction:column; gap:0.9rem; border:1px solid #ffffff20; border-radius:0.75rem; padding:1.25rem; }
    legend { padding:0 0.5rem; font-size:1.15rem; font-weight:600; }
    label { display:block; font-size:0.9rem; }
    input:not([type=checkbox]),select,textarea { display:block; width:100%; margin-top:0.35rem; padding:0.7rem; background:#252525; border:1px solid #ffffff20; border-radius:0.4rem; color:white; }
    .check { display:flex; align-items:center; gap:0.6rem; }
    p { font-size:0.8rem; color:#aaa; }
    summary { cursor:pointer; margin-bottom:1rem; }
    .secondary { border:1px solid #ffffff40; border-radius:0.4rem; padding:0.6rem; }
    input:focus-visible,select:focus-visible,textarea:focus-visible,button:focus-visible { outline:2px solid #d5ad42; outline-offset:2px; }
</style>
