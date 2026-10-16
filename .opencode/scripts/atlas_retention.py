#!/usr/bin/env python3
"""Join official per-video YouTube retention data to the PUBLISHED film's exact scene timeline.
Standard library only. Generates editorial observations, NOT a derived public YouTube score.
"""
import argparse, json, statistics
from datetime import datetime, timezone
from pathlib import Path

def read_json(path):return json.loads(Path(path).read_text(encoding='utf-8'))
def as_float(value):
    try:return float(value)
    except (ValueError,TypeError):return None

def extract_samples(api_report):
    headers=api_report.get('columnHeaders',[])
    names=[h['name'] if isinstance(h,dict) else h for h in headers]
    if 'elapsedVideoTimeRatio' not in names or 'audienceWatchRatio' not in names:
        raise ValueError('Required retention dimension/metric absent from columnHeaders')
    samples=[]
    for row in api_report.get('rows',[]):
        values=dict(zip(names,row))
        ratio=as_float(values.get('elapsedVideoTimeRatio'))
        watch=as_float(values.get('audienceWatchRatio'))
        if ratio is None or watch is None or not 0<ratio<=1 or watch<0:continue
        rel=as_float(values.get('relativeRetentionPerformance'))
        samples.append({'ratio':ratio,'watch_ratio':watch,'relative_retention':rel})
    samples.sort(key=lambda x:x['ratio'])
    unique={x['ratio']:x for x in samples}
    return [unique[k] for k in sorted(unique)]

def scene_for_time(scenes, second):
    for item in scenes:
        if item['start_second'] <= second < item['end_second']:
            return item
    if scenes and abs(second-scenes[-1]['end_second']) < 0.1:return scenes[-1]
    return None

def analyze(api_report, timeline, min_views=100, views=None):
    if api_report.get('video_id') != timeline.get('video_id'):
        raise ValueError('Analytics video_id does not match the published scene timeline video_id')
    duration=float(timeline['duration_seconds'])
    if duration<=0:raise ValueError('Invalid duration')
    scenes=sorted(timeline['scenes'], key=lambda s:s['start_second'])
    if not scenes or scenes[0]['start_second']<0 or scenes[-1]['end_second']>duration+0.1:
        raise ValueError('Invalid scene boundaries for video duration')
    for left,right in zip(scenes,scenes[1:]):
        if left['end_second']>right['start_second']+0.001:raise ValueError('Overlapping scenes')
    samples=extract_samples(api_report)
    data_sufficiency='unknown_views'
    if views is not None:
        data_sufficiency='provisional_low_views' if views<min_views else 'meets_editing_threshold'
    if len(samples)<2:
        return {'video_id':api_report['video_id'],'status':'not_available','reason':'Less than 2 valid measured points; do not zero-fill.','observations':[],'observed_points':len(samples),'data_sufficiency':data_sufficiency}
    observations=[]
    # Simple neighboring sample change is an internal editorial annotation, not a new channel-performance metric.
    for i in range(1,len(samples)):
        prev,cur=samples[i-1],samples[i]
        delta=cur['watch_ratio']-prev['watch_ratio']
        if delta>-0.065 and delta<0.055:continue
        start_s=prev['ratio']*duration
        end_s=cur['ratio']*duration
        sample_mid=(start_s+end_s)/2
        scene=scene_for_time(scenes,sample_mid)
        obs={
            'observation_id':f"{api_report['video_id']}-{i:03d}",
            'type':'localized_decline' if delta<0 else 'localized_rewatch_or_seek_increase',
            'bucket_start_second':round(start_s,3), 'bucket_end_second':round(end_s,3),
            'audience_watch_ratio_before':round(prev['watch_ratio'],5),
            'audience_watch_ratio_after':round(cur['watch_ratio'],5),
            'reported_absolute_change':round(delta,5),
            'relative_retention_at_end':cur['relative_retention'],
            'scene_id':scene.get('scene_id') if scene else None,
            'beat_id':scene.get('beat_id') if scene else None,
            'scene_narrative_function':scene.get('narrative_function') if scene else None,
            'scene_visual_device':scene.get('visual_device') if scene else None,
            'interpretation':'Hypothesis only; retention observations do not prove scene causality.',
        }
        observations.append(obs)
    observations=sorted(observations,key=lambda x:(-abs(x['reported_absolute_change']),x['bucket_end_second']))[:12]
    limitation=[
        'Official samples are normalized intervals, not individual-viewer sessions or frame-granular retention.',
        'Audience watch ratio can exceed 1 due to rewatch/seek behavior.',
        'Changes in traffic mix, topic, video age and seasonality may confound observations.',
        'Automated editorial alerts do not establish causation or viewer satisfaction.',
    ]
    if data_sufficiency != 'meets_editing_threshold':limitation.append('Data volume is unknown or low; treat insights as provisional.')
    return {
      'video_id':api_report['video_id'], 'status':'observed',
      'source':'YouTube Analytics reports.query: audienceWatchRatio / elapsedVideoTimeRatio',
      'retrieved_at':api_report.get('retrieved_at'), 'observation_period':{'start':api_report.get('start_date'),'end':api_report.get('end_date')},
      'video_duration_seconds':duration,'observed_points':len(samples),'views_for_context':views,
      'data_sufficiency':data_sufficiency,
      'scene_annotations':[{ 'scene_id':s['scene_id'],'start_second':s['start_second'],'end_second':s['end_second'],'narrative_function':s.get('narrative_function'),'visual_device':s.get('visual_device')} for s in scenes],
      'observations':observations, 'interpretation_limits':limitation,
      'editorial_instruction':'Review observations against video/audio before proposing a single-variable future episode test; do not autopromote.'
    }

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--report',required=True)
    p.add_argument('--timeline',required=True)
    p.add_argument('--out',required=True)
    p.add_argument('--views',type=int,default=None,help='Owner-authorized view count for data maturity context; omit if unavailable')
    p.add_argument('--min-views',type=int,default=100,help='Editorial review gate, not a platform-defined threshold')
    a=p.parse_args()
    result=analyze(read_json(a.report),read_json(a.timeline),a.min_views,a.views)
    pth=Path(a.out);pth.parent.mkdir(parents=True,exist_ok=True)
    pth.write_text(json.dumps(result,indent=2,ensure_ascii=False)+'\n',encoding='utf-8')
    print(f"Wrote {pth}: {result['status']}; {len(result['observations'])} editorial observations, {result['observed_points']} samples")
if __name__=='__main__':main()
