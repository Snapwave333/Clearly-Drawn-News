#!/usr/bin/env python3
"""Propose a HUMAN/INDEPENDENT-AGENT reviewed next-video creative experiment from retention observations.
Never modifies videos, uploads content, or treats observational retention data as causal proof.
"""
import argparse,json,uuid
from pathlib import Path

def choose(review):
    observations=review.get('observations',[])
    if review.get('status')!='observed' or review.get('data_sufficiency')!='meets_editing_threshold' or not observations:
        return {'status':'insufficient_evidence','reason':'No mature/adequate scene-level evidence; obtain valid views and retention before suggesting automatic trials.','experiment':None}
    chosen=next((o for o in observations if o.get('type')=='localized_decline'),observations[0])
    visual=(chosen.get('scene_visual_device') or '').lower()
    if any(x in visual for x in ['dense','diagram','exploded','variable']):
        var='diagram reveal density';change='Present one diagram relationship at a time with intentional 2-second reading holds.'
    elif 'intro' in (chosen.get('scene_narrative_function') or '').lower():
        var='hook visual premise';change='Begin with a clearly drawn mechanism/question before narration context.'
    else:
        var='scene explanation cadence';change='Split one dense narration beat into visual cause → pause → consequence.'
    return {'status':'proposal_only','experiment':{
        'experiment_id':'exp-'+uuid.uuid4().hex[:12],
        'source_video_id':review.get('video_id'), 'source_scene_id':chosen.get('scene_id'),
        'hypothesis':f'Future videos using {change.lower()} may better maintain attention during comparable explanatory beats; this is not established by a single retention curve.',
        'change_variable':var,'control_rule':'Retain the current production bible pacing for matching explanation beats.',
        'variant_rule':change,'comparison_rule':'Use future episodes of similar topical domain, duration and traffic context; note confounders and never claim randomized causality unless randomized.',
        'primary_observed_metric':'audienceWatchRatio','evaluation_window_days':14,
        'minimum_suggested_views_per_episode':100,
        'guardrails':['factual accuracy invariant','AI credit and platform disclosure invariant','originality review mandatory','readability at mobile size','one creative variable per trial','no misleading packaging or fake traffic'],
        'status':'proposed','owner_approval_required':True,
        'promotion_rule':'Independent auditor verifies comparable mature observations and no factual/originality regression; default to no automatic promotion.',
        'revert_rule':'Roll back if verified degradation, confounding or missing data; do not repeat a failed template.',
        'note':'YouTube native Studio thumbnail experiments are not automatically driven via public API.'
      }}

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--review',required=True);p.add_argument('--out',required=True);a=p.parse_args()
    data=choose(json.loads(Path(a.review).read_text(encoding='utf-8')))
    target=Path(a.out);target.parent.mkdir(parents=True,exist_ok=True);target.write_text(json.dumps(data,indent=2)+'\n',encoding='utf-8')
    print('Wrote',target,'status',data['status'])
if __name__=='__main__':main()
