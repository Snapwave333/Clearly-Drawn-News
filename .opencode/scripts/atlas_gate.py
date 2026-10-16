#!/usr/bin/env python3
"""Reference fail-closed publish-gate validator. Not a full rights/fact quality engine."""
import argparse,json,sys
from pathlib import Path

def validate(manifest,public=False):
    errors=[]
    approved=manifest.get('approvals',{})
    for kind in ('facts','rights','quality'):
        if approved.get(kind) is not True:errors.append(kind+' independent approval missing')
    if not manifest.get('sources'):errors.append('no sources ledger')
    if not manifest.get('assets'):errors.append('no immutable asset manifests')
    if not manifest.get('disclosure',{}).get('credits_in_video',False):errors.append('AI-production credits missing')
    if manifest.get('disclosure',{}).get('realistic_synthetic_media',False) and not manifest.get('disclosure',{}).get('platform_disclosure',False):
        errors.append('realistic synthetic media missing YouTube disclosure')
    if public:
        if approved.get('user_publish_consent') is not True:errors.append('user public-publishing consent absent')
        if manifest.get('publish_mode')!='autonomous_opt_in':errors.append('mode is not autonomous_opt_in')
        if manifest.get('state')!='QUALITY_PASSED':errors.append('episode has not reached QUALITY_PASSED')
    return errors

def main():
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument('manifest');ap.add_argument('--public',action='store_true')
    a=ap.parse_args();data=json.loads(Path(a.manifest).read_text(encoding='utf-8'))
    errors=validate(data,public=a.public)
    if errors:
        print('BLOCKED:');[print(' -',e) for e in errors];sys.exit(2)
    print('GATE CONDITIONS PRESENT: permits next workflow step; does NOT itself certify facts/rights or upload.')
if __name__=='__main__':main()
