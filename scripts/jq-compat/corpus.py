#!/usr/bin/env python3
"""The real-world jq corpus (docs/design/jq.md, release gate 1): programs as
people run them on the output of kubectl, the AWS CLI, gh, Docker,
Terraform and npm, on logs and on data from Stack Overflow's most asked jq
questions, beside the 170 of idioms.py. Each becomes a compatibility case
under crates/cli/tests/compat/jq/ (args and stdin); scripts/compat_record.sh
records what jq 1.8.2 gives.

    python3 -I scripts/jq-compat/corpus.py crates/cli/tests/compat/jq
"""
import json, os, sys

def j(v):
    return json.dumps(v, ensure_ascii=False)

def lines(vs):
    return "".join(json.dumps(v, ensure_ascii=False) + "\n" for v in vs)

# --- kubectl -------------------------------------------------------------

def pod(name, ns, phase, node, containers, labels=None, created="2026-10-30T08:15:00Z",
        restarts=None, ip=None, owner=None, conditions=None):
    restarts = restarts or [0] * len(containers)
    return {
        "apiVersion": "v1", "kind": "Pod",
        "metadata": {"name": name, "namespace": ns, "labels": labels or {},
                     "creationTimestamp": created,
                     **({"ownerReferences": [{"kind": owner[0], "name": owner[1]}]} if owner else {})},
        "spec": {"nodeName": node, "containers": [
            {"name": c[0], "image": c[1],
             **({"resources": c[2]} if len(c) > 2 else {})} for c in containers]},
        "status": {"phase": phase, **({"podIP": ip} if ip else {}),
                   "conditions": conditions or [{"type": "Ready", "status": "True" if phase == "Running" else "False"}],
                   "containerStatuses": [
                       {"name": c[0], "ready": phase == "Running", "restartCount": r,
                        "image": c[1]} for c, r in zip(containers, restarts)]},
    }

PODS = {"apiVersion": "v1", "kind": "List", "items": [
    pod("web-7d9f-abc", "prod", "Running", "node-a",
        [("web", "nginx:1.27", {"requests": {"cpu": "250m", "memory": "128Mi"}, "limits": {"cpu": "500m", "memory": "256Mi"}})],
        {"app": "web", "tier": "frontend"}, "2026-10-30T08:15:00Z", ip="10.0.0.12", owner=("ReplicaSet", "web-7d9f")),
    pod("web-7d9f-def", "prod", "Running", "node-b",
        [("web", "nginx:1.27", {"requests": {"cpu": "250m", "memory": "128Mi"}})],
        {"app": "web", "tier": "frontend"}, "2026-10-30T08:16:10Z", ip="10.0.1.7", owner=("ReplicaSet", "web-7d9f")),
    pod("api-55c1-x9", "prod", "CrashLoopBackOff", "node-b",
        [("api", "ghcr.io/acme/api:2.4.1", {"limits": {"memory": "1Gi"}}), ("envoy", "envoyproxy/envoy:v1.31.0")],
        {"app": "api", "tier": "backend"}, "2026-10-29T22:01:30Z", restarts=[17, 0], owner=("ReplicaSet", "api-55c1")),
    pod("worker-0", "batch", "Running", "node-c",
        [("worker", "ghcr.io/acme/worker:1.9.0", {"requests": {"cpu": "1", "memory": "2Gi"}})],
        {"app": "worker"}, "2026-11-01T03:00:00Z", restarts=[2], ip="10.0.2.3", owner=("StatefulSet", "worker")),
    pod("migrate-28431", "batch", "Succeeded", "node-a", [("migrate", "ghcr.io/acme/api:2.4.1")],
        {"job-name": "migrate-28431"}, "2026-11-02T00:00:00Z", owner=("Job", "migrate-28431")),
    pod("debug-shell", "default", "Pending", None, [("sh", "busybox:1.36")], None, "2026-11-03T01:20:00Z",
        conditions=[{"type": "PodScheduled", "status": "False", "reason": "Unschedulable"}]),
]}

NODES = {"items": [
    {"metadata": {"name": "node-a", "labels": {"kubernetes.io/arch": "amd64", "node.kubernetes.io/instance-type": "m6i.large", "topology.kubernetes.io/zone": "eu-west-1a"}},
     "spec": {"taints": [{"key": "dedicated", "value": "web", "effect": "NoSchedule"}]},
     "status": {"capacity": {"cpu": "2", "memory": "8022356Ki", "pods": "29"}, "allocatable": {"cpu": "1930m", "memory": "7100980Ki"},
                "conditions": [{"type": "Ready", "status": "True"}, {"type": "MemoryPressure", "status": "False"}],
                "nodeInfo": {"kubeletVersion": "v1.31.2", "osImage": "Bottlerocket OS 1.26.1", "containerRuntimeVersion": "containerd://1.7.22"},
                "addresses": [{"type": "InternalIP", "address": "172.31.10.4"}, {"type": "Hostname", "address": "ip-172-31-10-4"}]}},
    {"metadata": {"name": "node-b", "labels": {"kubernetes.io/arch": "arm64", "node.kubernetes.io/instance-type": "m7g.large", "topology.kubernetes.io/zone": "eu-west-1b"}},
     "spec": {},
     "status": {"capacity": {"cpu": "2", "memory": "8022356Ki", "pods": "29"}, "allocatable": {"cpu": "1930m", "memory": "7100980Ki"},
                "conditions": [{"type": "Ready", "status": "False", "reason": "KubeletNotReady"}, {"type": "MemoryPressure", "status": "True"}],
                "nodeInfo": {"kubeletVersion": "v1.30.6", "osImage": "Bottlerocket OS 1.26.1", "containerRuntimeVersion": "containerd://1.7.22"},
                "addresses": [{"type": "InternalIP", "address": "172.31.20.9"}]}},
    {"metadata": {"name": "node-c", "labels": {"kubernetes.io/arch": "amd64", "node.kubernetes.io/instance-type": "c6i.2xlarge", "topology.kubernetes.io/zone": "eu-west-1a"}},
     "spec": {"unschedulable": True},
     "status": {"capacity": {"cpu": "8", "memory": "16110052Ki", "pods": "58"}, "allocatable": {"cpu": "7910m", "memory": "15000000Ki"},
                "conditions": [{"type": "Ready", "status": "True"}],
                "nodeInfo": {"kubeletVersion": "v1.31.2", "osImage": "Amazon Linux 2023", "containerRuntimeVersion": "containerd://1.7.22"},
                "addresses": [{"type": "InternalIP", "address": "172.31.30.2"}]}},
]}

DEPLOYS = {"items": [
    {"metadata": {"name": "web", "namespace": "prod", "annotations": {"deployment.kubernetes.io/revision": "14"}},
     "spec": {"replicas": 3, "template": {"spec": {"containers": [{"name": "web", "image": "nginx:1.27", "env": [{"name": "MODE", "value": "prod"}, {"name": "TOKEN", "valueFrom": {"secretKeyRef": {"name": "web", "key": "token"}}}]}]}}},
     "status": {"replicas": 3, "readyReplicas": 3, "updatedReplicas": 3, "availableReplicas": 3}},
    {"metadata": {"name": "api", "namespace": "prod", "annotations": {"deployment.kubernetes.io/revision": "3"}},
     "spec": {"replicas": 2, "template": {"spec": {"containers": [{"name": "api", "image": "ghcr.io/acme/api:2.4.1"}, {"name": "envoy", "image": "envoyproxy/envoy:v1.31.0"}]}}},
     "status": {"replicas": 2, "readyReplicas": 0, "updatedReplicas": 2, "unavailableReplicas": 2,
                "conditions": [{"type": "Available", "status": "False", "reason": "MinimumReplicasUnavailable"}]}},
    {"metadata": {"name": "admin", "namespace": "internal"},
     "spec": {"replicas": 0, "template": {"spec": {"containers": [{"name": "admin", "image": "ghcr.io/acme/admin:0.3.0"}]}}},
     "status": {}},
]}

EVENTS = {"items": [
    {"type": "Warning", "reason": "BackOff", "count": 41, "lastTimestamp": "2026-11-03T01:00:00Z", "involvedObject": {"kind": "Pod", "name": "api-55c1-x9"}, "message": "Back-off restarting failed container api"},
    {"type": "Normal", "reason": "Scheduled", "count": 1, "lastTimestamp": "2026-11-02T00:00:01Z", "involvedObject": {"kind": "Pod", "name": "migrate-28431"}, "message": "Successfully assigned batch/migrate-28431 to node-a"},
    {"type": "Warning", "reason": "FailedScheduling", "count": 3, "lastTimestamp": "2026-11-03T01:21:00Z", "involvedObject": {"kind": "Pod", "name": "debug-shell"}, "message": "0/3 nodes are available: 1 node(s) were unschedulable"},
    {"type": "Warning", "reason": "BackOff", "count": 5, "lastTimestamp": "2026-11-02T23:30:00Z", "involvedObject": {"kind": "Pod", "name": "worker-0"}, "message": "Back-off pulling image"},
]}

SECRET = {"apiVersion": "v1", "kind": "Secret", "metadata": {"name": "db", "namespace": "prod"}, "type": "Opaque",
          "data": {"username": "YXBw", "password": "czNjcjN0IQ==", "url": "cG9zdGdyZXM6Ly9kYi5pbnRlcm5hbDo1NDMyL2FwcA=="}}

CM = {"apiVersion": "v1", "kind": "ConfigMap", "metadata": {"name": "billing", "namespace": "prod", "labels": {"app": "billing"}},
      "data": {"LOG_LEVEL": "info", "ENABLE_V2": "no", "VERSION": "1.10", "settings.json": "{\"retries\": 3, \"timeout_ms\": 2500}"}}

SVCS = {"items": [
    {"metadata": {"name": "web", "namespace": "prod"}, "spec": {"type": "LoadBalancer", "clusterIP": "172.20.1.10", "ports": [{"name": "http", "port": 80, "targetPort": 8080, "protocol": "TCP"}, {"name": "https", "port": 443, "targetPort": 8443, "protocol": "TCP"}], "selector": {"app": "web"}},
     "status": {"loadBalancer": {"ingress": [{"hostname": "a1b2.elb.eu-west-1.amazonaws.com"}]}}},
    {"metadata": {"name": "api", "namespace": "prod"}, "spec": {"type": "ClusterIP", "clusterIP": "172.20.4.2", "ports": [{"port": 9000, "targetPort": 9000, "protocol": "TCP"}], "selector": {"app": "api"}}, "status": {"loadBalancer": {}}},
    {"metadata": {"name": "dns", "namespace": "kube-system"}, "spec": {"type": "ClusterIP", "clusterIP": "172.20.0.10", "ports": [{"name": "dns", "port": 53, "protocol": "UDP"}, {"name": "dns-tcp", "port": 53, "protocol": "TCP"}]}, "status": {}},
]}

KUBECONFIG = {"apiVersion": "v1", "kind": "Config", "current-context": "prod",
              "clusters": [{"name": "prod", "cluster": {"server": "https://prod.k8s.acme.test"}}, {"name": "staging", "cluster": {"server": "https://staging.k8s.acme.test", "insecure-skip-tls-verify": True}}],
              "contexts": [{"name": "prod", "context": {"cluster": "prod", "user": "admin", "namespace": "prod"}}, {"name": "staging", "context": {"cluster": "staging", "user": "dev"}}],
              "users": [{"name": "admin", "user": {"token": "x"}}, {"name": "dev", "user": {"exec": {"command": "aws"}}}]}

K = j(PODS)
CASES = [
    ("k8s-pods-table", ["-r", '.items[] | [.metadata.namespace, .metadata.name, .status.phase] | @tsv'], K),
    ("k8s-pods-per-node", ["-c", '[.items[] | select(.spec.nodeName) | .spec.nodeName] | group_by(.) | map({node: .[0], pods: length})'], K),
    ("k8s-pods-not-ready", ["-r", '.items[] | select(any(.status.conditions[]; .type == "Ready" and .status != "True")) | .metadata.name'], K),
    ("k8s-pods-restarts-total", ["-c", '[.items[] | {name: .metadata.name, restarts: (.status.containerStatuses | map(.restartCount) | add)}] | sort_by(-.restarts) | .[0:2]'], K),
    ("k8s-pods-images-unique", ["-r", '[.items[].spec.containers[].image] | unique[]'], K),
    ("k8s-pods-image-tags", ["-c", '[.items[].spec.containers[].image | split(":") | {repo: .[0], tag: (.[1] // "latest")}] | unique_by(.repo)'], K),
    ("k8s-pods-by-owner", ["-c", 'reduce .items[] as $p ({}; .[($p.metadata.ownerReferences[0].kind // "none")] += [$p.metadata.name])'], K),
    ("k8s-pods-label-filter", ["-r", '.items[] | select(.metadata.labels.tier == "frontend") | "\\(.metadata.name) \\(.status.podIP // "-")"'], K),
    ("k8s-pods-label-selector-string", ["-r", '.items[] | .metadata.labels | to_entries | map("\\(.key)=\\(.value)") | join(",") | select(length > 0)'], K),
    ("k8s-pods-oldest", ["-r", '.items | min_by(.metadata.creationTimestamp) | .metadata.name'], K),
    ("k8s-pods-newest-3", ["-r", '.items | sort_by(.metadata.creationTimestamp) | reverse | .[:3][] | .metadata.name'], K),
    ("k8s-pods-age-hours", ["-c", '[.items[] | {name: .metadata.name, hours: ((1762142400 - (.metadata.creationTimestamp | fromdateiso8601)) / 3600 | floor)}]'], K),
    ("k8s-pods-no-limits", ["-r", '.items[] | .metadata.name as $p | .spec.containers[] | select(.resources.limits == null) | "\\($p)/\\(.name)"'], K),
    ("k8s-pods-memory-requests", ["-c", '[.items[].spec.containers[].resources.requests.memory // empty] | map(if endswith("Gi") then (rtrimstr("Gi") | tonumber * 1024) else (rtrimstr("Mi") | tonumber) end) | add'], K),
    ("k8s-pods-cpu-millicores", ["-c", '[.items[].spec.containers[].resources.requests.cpu // empty | if endswith("m") then rtrimstr("m") | tonumber else tonumber * 1000 end] | {total: add, max: max}'], K),
    ("k8s-pods-phase-counts", ["-c", '.items | group_by(.status.phase) | map({(.[0].status.phase): length}) | add'], K),
    ("k8s-pods-namespaces", ["-r", '[.items[].metadata.namespace] | unique | join(" ")'], K),
    ("k8s-pods-pending-reason", ["-r", '.items[] | select(.status.phase == "Pending") | .status.conditions[] | select(.status == "False") | "\\(.type): \\(.reason)"'], K),
    ("k8s-pods-sidecars", ["-c", '[.items[] | select(.spec.containers | length > 1) | {pod: .metadata.name, containers: [.spec.containers[].name]}]'], K),
    ("k8s-pods-delete-commands", ["-r", '.items[] | select(.status.phase == "Succeeded") | "kubectl delete pod -n \\(.metadata.namespace) \\(.metadata.name)"'], K),
    ("k8s-pods-to-map", ["-c", '.items | map({key: .metadata.name, value: .status.phase}) | from_entries'], K),
    ("k8s-pods-paths-images", ["-c", '[paths(type == "string" and test("^ghcr.io/"))] | length'], K),
    ("k8s-pods-strip-status", ["-c", '.items[0] | del(.status) | .metadata |= with_entries(select(.key | IN("name", "namespace")))'], K),
    ("k8s-pods-owner-names", ["-r", '.items[] | .metadata.ownerReferences[]? | select(.kind == "ReplicaSet") | .name | sub("-[a-z0-9]+$"; "")'], K),
    ("k8s-pods-first-ready-ip", ["-r", 'first(.items[] | select(.status.phase == "Running") | .status.podIP)'], K),
    ("k8s-pods-any-crash", ["-e", 'any(.items[]; .status.phase == "CrashLoopBackOff")'], K),
    ("k8s-pods-all-scheduled", ["-e", 'all(.items[]; .spec.nodeName != null)'], K),
    ("k8s-pods-statefulset-ordinal", ["-c", '[.items[] | .metadata.name | capture("-(?<n>[0-9]+)$") | .n | tonumber]'], K),
    ("k8s-pods-limit-ratio", ["-c", '[.items[].spec.containers[] | select(.resources.limits.memory and .resources.requests.memory) | {name, ratio: ((.resources.limits.memory | rtrimstr("Mi") | tonumber) / (.resources.requests.memory | rtrimstr("Mi") | tonumber))}]'], K),
    ("k8s-pods-walk-redact", ["-c", '.items[2] | walk(if type == "object" and has("image") then .image = "<redacted>" else . end) | .spec.containers'], K),
    ("k8s-nodes-capacity", ["-r", '.items[] | [.metadata.name, .status.capacity.cpu, (.status.capacity.memory | rtrimstr("Ki") | tonumber / 1048576 | floor | tostring + "Gi")] | @tsv'], j(NODES)),
    ("k8s-nodes-not-ready", ["-r", '.items[] | select(.status.conditions[] | .type == "Ready" and .status != "True") | .metadata.name'], j(NODES)),
    ("k8s-nodes-pressure", ["-c", '[.items[] | {name: .metadata.name, pressure: [.status.conditions[] | select(.type != "Ready" and .status == "True") | .type]} | select(.pressure | length > 0)]'], j(NODES)),
    ("k8s-nodes-taints", ["-r", '.items[] | select(.spec.taints) | .metadata.name + ": " + (.spec.taints | map("\\(.key)=\\(.value):\\(.effect)") | join(", "))'], j(NODES)),
    ("k8s-nodes-by-zone", ["-c", '.items | group_by(.metadata.labels["topology.kubernetes.io/zone"]) | map({zone: .[0].metadata.labels["topology.kubernetes.io/zone"], nodes: map(.metadata.name)})'], j(NODES)),
    ("k8s-nodes-versions", ["-c", '[.items[].status.nodeInfo.kubeletVersion] | unique | {versions: ., skew: (length > 1)}'], j(NODES)),
    ("k8s-nodes-internal-ip", ["-r", '.items[] | "\\(.metadata.name)\\t\\(.status.addresses[] | select(.type == "InternalIP") | .address)"'], j(NODES)),
    ("k8s-nodes-cordoned", ["-r", '.items[] | select(.spec.unschedulable // false) | .metadata.name'], j(NODES)),
    ("k8s-nodes-arch-count", ["-c", 'reduce .items[] as $n ({}; .[$n.metadata.labels["kubernetes.io/arch"]] += 1)'], j(NODES)),
    ("k8s-nodes-total-cpu", ["-c", '[.items[].status.allocatable.cpu | if endswith("m") then (rtrimstr("m") | tonumber / 1000) else tonumber end] | add'], j(NODES)),
    ("k8s-deploy-unavailable", ["-r", '.items[] | select((.status.readyReplicas // 0) < .spec.replicas) | "\\(.metadata.namespace)/\\(.metadata.name) \\(.status.readyReplicas // 0)/\\(.spec.replicas)"'], j(DEPLOYS)),
    ("k8s-deploy-images", ["-c", '.items | map({(.metadata.name): [.spec.template.spec.containers[].image]}) | add'], j(DEPLOYS)),
    ("k8s-deploy-revision", ["-c", '[.items[] | {name: .metadata.name, rev: (.metadata.annotations["deployment.kubernetes.io/revision"] // "0" | tonumber)}] | max_by(.rev)'], j(DEPLOYS)),
    ("k8s-deploy-secret-refs", ["-c", '[.items[].spec.template.spec.containers[].env[]? | select(.valueFrom.secretKeyRef) | .valueFrom.secretKeyRef]'], j(DEPLOYS)),
    ("k8s-deploy-scaled-to-zero", ["-r", '.items[] | select(.spec.replicas == 0) | .metadata.name'], j(DEPLOYS)),
    ("k8s-deploy-set-image", ["-c", '.items[1] | .spec.template.spec.containers |= map(if .name == "api" then .image = "ghcr.io/acme/api:2.4.2" else . end) | .spec.template.spec.containers'], j(DEPLOYS)),
    ("k8s-deploy-conditions", ["-r", '.items[] | .metadata.name as $n | .status.conditions[]? | "\\($n): \\(.type)=\\(.status) (\\(.reason))"'], j(DEPLOYS)),
    ("k8s-events-warnings", ["-r", '.items | map(select(.type == "Warning")) | sort_by(.lastTimestamp) | reverse[] | "\\(.lastTimestamp) \\(.involvedObject.name): \\(.message)"'], j(EVENTS)),
    ("k8s-events-by-reason", ["-c", '.items | group_by(.reason) | map({reason: .[0].reason, total: (map(.count) | add)}) | sort_by(-.total)'], j(EVENTS)),
    ("k8s-secret-decode", ["-c", '.data | map_values(@base64d)'], j(SECRET)),
    ("k8s-secret-decode-one", ["-r", '.data.url | @base64d'], j(SECRET)),
    ("k8s-secret-keys", ["-r", '.data | keys[]'], j(SECRET)),
    ("k8s-secret-create-manifest", ["-c", '{apiVersion: "v1", kind: "Secret", metadata: {name: "new"}, data: ({user: "admin", pass: "p@ss"} | map_values(@base64))}'], "null"),
    ("k8s-configmap-json-field", ["-c", '.data["settings.json"] | fromjson | .timeout_ms / 1000'], j(CM)),
    ("k8s-configmap-env-file", ["-r", '.data | to_entries[] | select(.key | test("^[A-Z_]+$")) | "\\(.key)=\\(.value)"'], j(CM)),
    ("k8s-configmap-patch", ["-c", '.data.LOG_LEVEL = "debug" | .metadata |= del(.labels) | {metadata, data: .data | {LOG_LEVEL}}'], j(CM)),
    ("k8s-svc-endpoints", ["-r", '.items[] | select(.spec.type == "LoadBalancer") | .status.loadBalancer.ingress[0] | .hostname // .ip'], j(SVCS)),
    ("k8s-svc-ports", ["-r", '.items[] | .metadata.name as $s | .spec.ports[] | "\\($s):\\(.port)/\\(.protocol)"'], j(SVCS)),
    ("k8s-svc-no-selector", ["-r", '.items[] | select(.spec.selector == null) | .metadata.namespace + "/" + .metadata.name'], j(SVCS)),
    ("k8s-svc-port-map", ["-c", '[.items[] | {(.metadata.name): (.spec.ports | map({(.name // (.port | tostring)): .port}) | add)}] | add'], j(SVCS)),
    ("k8s-kubeconfig-current", ["-r", '."current-context" as $c | .contexts[] | select(.name == $c) | .context | "\\(.cluster) \\(.namespace // "default")"'], j(KUBECONFIG)),
    ("k8s-kubeconfig-servers", ["-c", '.clusters | map({(.name): .cluster.server}) | add'], j(KUBECONFIG)),
    ("k8s-kubeconfig-insecure", ["-r", '.clusters[] | select(.cluster["insecure-skip-tls-verify"]) | .name'], j(KUBECONFIG)),
    ("k8s-kubeconfig-users-auth", ["-c", '[.users[] | {name, auth: (.user | keys[0])}]'], j(KUBECONFIG)),
]

# --- AWS CLI -------------------------------------------------------------

EC2 = {"Reservations": [
    {"ReservationId": "r-01", "Instances": [
        {"InstanceId": "i-0a1", "InstanceType": "t3.micro", "State": {"Code": 16, "Name": "running"}, "PrivateIpAddress": "172.31.1.5", "PublicIpAddress": "54.1.2.3",
         "LaunchTime": "2026-10-01T10:00:00+00:00", "Placement": {"AvailabilityZone": "eu-west-1a"}, "ImageId": "ami-111",
         "SecurityGroups": [{"GroupId": "sg-1", "GroupName": "bastion"}], "BlockDeviceMappings": [{"DeviceName": "/dev/xvda", "Ebs": {"VolumeId": "vol-1"}}],
         "Tags": [{"Key": "Name", "Value": "bastion"}, {"Key": "env", "Value": "prod"}, {"Key": "owner", "Value": "ops"}]},
        {"InstanceId": "i-0a2", "InstanceType": "m6i.large", "State": {"Code": 80, "Name": "stopped"}, "PrivateIpAddress": "172.31.2.9",
         "LaunchTime": "2026-06-15T08:30:00+00:00", "Placement": {"AvailabilityZone": "eu-west-1b"}, "ImageId": "ami-222",
         "SecurityGroups": [{"GroupId": "sg-2", "GroupName": "workers"}, {"GroupId": "sg-3", "GroupName": "ssh"}], "BlockDeviceMappings": [],
         "Tags": [{"Key": "Name", "Value": "worker-1"}, {"Key": "env", "Value": "staging"}]}]},
    {"ReservationId": "r-02", "Instances": [
        {"InstanceId": "i-0b1", "InstanceType": "m6i.large", "State": {"Code": 16, "Name": "running"}, "PrivateIpAddress": "172.31.3.3",
         "LaunchTime": "2026-11-02T22:10:00+00:00", "Placement": {"AvailabilityZone": "eu-west-1a"}, "ImageId": "ami-222",
         "SecurityGroups": [{"GroupId": "sg-2", "GroupName": "workers"}], "BlockDeviceMappings": [{"DeviceName": "/dev/xvda", "Ebs": {"VolumeId": "vol-3"}}, {"DeviceName": "/dev/sdf", "Ebs": {"VolumeId": "vol-4"}}]}]},
]}

S3 = {"Contents": [
    {"Key": "logs/2026/11/01/app.log.gz", "Size": 1048576, "LastModified": "2026-11-01T23:59:59.000Z", "StorageClass": "STANDARD", "ETag": "\"a1\""},
    {"Key": "logs/2026/11/02/app.log.gz", "Size": 2097152, "LastModified": "2026-11-02T23:59:59.000Z", "StorageClass": "STANDARD", "ETag": "\"a2\""},
    {"Key": "backups/db-2026-10-31.sql.gz", "Size": 524288000, "LastModified": "2026-10-31T03:00:00.000Z", "StorageClass": "GLACIER", "ETag": "\"b1\""},
    {"Key": "index.html", "Size": 5120, "LastModified": "2026-09-10T12:00:00.000Z", "StorageClass": "STANDARD", "ETag": "\"c1\""},
], "KeyCount": 4}

IAM = {"Policy": {"PolicyName": "deploy", "DefaultVersionId": "v3"}, "PolicyVersion": {"Document": {"Version": "2012-10-17", "Statement": [
    {"Sid": "S3", "Effect": "Allow", "Action": ["s3:GetObject", "s3:PutObject"], "Resource": "arn:aws:s3:::acme-artifacts/*"},
    {"Sid": "Deny", "Effect": "Deny", "Action": "iam:*", "Resource": "*"},
    {"Effect": "Allow", "Action": "ecr:*", "Resource": ["arn:aws:ecr:eu-west-1:123456789012:repository/api", "arn:aws:ecr:eu-west-1:123456789012:repository/web"]},
]}}}

LAMBDA = {"Functions": [
    {"FunctionName": "resize", "Runtime": "python3.12", "MemorySize": 512, "Timeout": 30, "LastModified": "2026-10-20T10:00:00.000+0000", "Architectures": ["arm64"], "Environment": {"Variables": {"BUCKET": "img", "QUALITY": "80"}}},
    {"FunctionName": "notify", "Runtime": "nodejs18.x", "MemorySize": 128, "Timeout": 3, "LastModified": "2025-03-01T09:00:00.000+0000", "Architectures": ["x86_64"]},
    {"FunctionName": "etl", "Runtime": "python3.9", "MemorySize": 3008, "Timeout": 900, "LastModified": "2026-08-05T18:30:00.000+0000", "Architectures": ["x86_64"]},
]}

CFN = {"Stacks": [
    {"StackName": "network", "StackStatus": "CREATE_COMPLETE", "CreationTime": "2025-01-10T10:00:00Z", "Outputs": [{"OutputKey": "VpcId", "OutputValue": "vpc-1"}, {"OutputKey": "SubnetIds", "OutputValue": "subnet-a,subnet-b"}], "Parameters": [{"ParameterKey": "Cidr", "ParameterValue": "10.0.0.0/16"}], "Tags": []},
    {"StackName": "api", "StackStatus": "UPDATE_ROLLBACK_COMPLETE", "CreationTime": "2025-06-01T10:00:00Z", "Outputs": [{"OutputKey": "Url", "OutputValue": "https://api.acme.test"}], "Parameters": [], "Tags": [{"Key": "team", "Value": "core"}]},
    {"StackName": "old-cache", "StackStatus": "DELETE_FAILED", "CreationTime": "2023-02-01T10:00:00Z", "Tags": [{"Key": "team", "Value": "core"}]},
]}

STS = {"UserId": "AROAEXAMPLE:alice", "Account": "123456789012", "Arn": "arn:aws:sts::123456789012:assumed-role/Admin/alice"}
CREDS = {"Credentials": {"AccessKeyId": "ASIAEXAMPLE", "SecretAccessKey": "secret/key+abc", "SessionToken": "token==", "Expiration": "2026-11-03T12:00:00+00:00"}}
METRICS = {"Label": "CPUUtilization", "Datapoints": [
    {"Timestamp": "2026-11-03T00:10:00Z", "Average": 41.2, "Maximum": 88.0, "Unit": "Percent"},
    {"Timestamp": "2026-11-03T00:00:00Z", "Average": 12.5, "Maximum": 30.1, "Unit": "Percent"},
    {"Timestamp": "2026-11-03T00:05:00Z", "Average": 63.75, "Maximum": 97.3, "Unit": "Percent"}]}
ECS = {"services": [
    {"serviceName": "web", "desiredCount": 3, "runningCount": 3, "pendingCount": 0, "deployments": [{"status": "PRIMARY", "taskDefinition": "arn:aws:ecs:eu-west-1:123:task-definition/web:42", "rolloutState": "COMPLETED"}]},
    {"serviceName": "api", "desiredCount": 2, "runningCount": 1, "pendingCount": 1, "deployments": [{"status": "PRIMARY", "taskDefinition": "arn:aws:ecs:eu-west-1:123:task-definition/api:7", "rolloutState": "IN_PROGRESS"}, {"status": "ACTIVE", "taskDefinition": "arn:aws:ecs:eu-west-1:123:task-definition/api:6", "rolloutState": "COMPLETED"}]}]}
R53 = {"ResourceRecordSets": [
    {"Name": "acme.test.", "Type": "A", "AliasTarget": {"DNSName": "a1b2.elb.amazonaws.com."}},
    {"Name": "acme.test.", "Type": "MX", "TTL": 300, "ResourceRecords": [{"Value": "10 mx1.acme.test."}, {"Value": "20 mx2.acme.test."}]},
    {"Name": "www.acme.test.", "Type": "CNAME", "TTL": 60, "ResourceRecords": [{"Value": "acme.test."}]},
    {"Name": "_dmarc.acme.test.", "Type": "TXT", "TTL": 3600, "ResourceRecords": [{"Value": "\"v=DMARC1; p=reject\""}]}]}
SQS = {"Messages": [
    {"MessageId": "m1", "ReceiptHandle": "rh1", "Body": "{\"order\": 1001, \"total\": 25.5}", "Attributes": {"ApproximateReceiveCount": "1"}},
    {"MessageId": "m2", "ReceiptHandle": "rh2", "Body": "{\"order\": 1002, \"total\": 99}", "Attributes": {"ApproximateReceiveCount": "4"}}]}

E = j(EC2)
CASES += [
    ("aws-ec2-table", ["-r", '.Reservations[].Instances[] | [.InstanceId, .InstanceType, .State.Name, (.Tags // [] | from_entries | .Name // "")] | @tsv'], E),
    ("aws-ec2-running-ids", ["-r", '[.Reservations[].Instances[] | select(.State.Name == "running") | .InstanceId] | join(" ")'], E),
    ("aws-ec2-tag-lookup", ["-r", '.Reservations[].Instances[] | (.Tags // [] | map({(.Key): .Value}) | add // {}) as $t | "\\(.InstanceId) env=\\($t.env // "none")"'], E),
    ("aws-ec2-untagged", ["-r", '.Reservations[].Instances[] | select(.Tags == null or (.Tags | map(.Key) | index("Name") | not)) | .InstanceId'], E),
    ("aws-ec2-by-type", ["-c", '[.Reservations[].Instances[]] | group_by(.InstanceType) | map({type: .[0].InstanceType, count: length})'], E),
    ("aws-ec2-by-az", ["-c", 'reduce .Reservations[].Instances[] as $i ({}; .[$i.Placement.AvailabilityZone] += [$i.InstanceId])'], E),
    ("aws-ec2-public-ips", ["-r", '.Reservations[].Instances[].PublicIpAddress // empty'], E),
    ("aws-ec2-security-groups", ["-c", '[.Reservations[].Instances[].SecurityGroups[].GroupName] | unique'], E),
    ("aws-ec2-volumes", ["-c", '[.Reservations[].Instances[] | {id: .InstanceId, volumes: [.BlockDeviceMappings[].Ebs.VolumeId]}]'], E),
    ("aws-ec2-launched-since", ["-r", '.Reservations[].Instances[] | select(.LaunchTime | sub("\\\\+00:00$"; "Z") | fromdate > 1761955200) | .InstanceId'], E),
    ("aws-ec2-ssh-config", ["-r", '.Reservations[].Instances[] | select(.State.Name == "running") | "Host \\(.Tags // [] | from_entries | .Name // .InstanceId)\\n  HostName \\(.PrivateIpAddress)"'], E),
    ("aws-ec2-count", ["-c", '{total: [.Reservations[].Instances[]] | length, running: [.Reservations[].Instances[] | select(.State.Code == 16)] | length}'], E),
    ("aws-ec2-amis", ["-r", '[.Reservations[].Instances[].ImageId] | unique | .[]'], E),
    ("aws-ec2-flatten-tags-csv", ["-r", '.Reservations[].Instances[] | [.InstanceId] + [.Tags[]? | .Value] | @csv'], E),
    ("aws-s3-total-size", ["-c", '[.Contents[].Size] | add / 1048576 | floor'], j(S3)),
    ("aws-s3-largest", ["-r", '.Contents | max_by(.Size) | "\\(.Key) \\(.Size)"'], j(S3)),
    ("aws-s3-prefix", ["-r", '.Contents[] | select(.Key | startswith("logs/")) | .Key'], j(S3)),
    ("aws-s3-by-class", ["-c", '.Contents | group_by(.StorageClass) | map({(.[0].StorageClass): (map(.Size) | add)}) | add'], j(S3)),
    ("aws-s3-older-than", ["-r", '.Contents[] | select(.LastModified | .[0:19] + "Z" | fromdate < 1761955200) | .Key'], j(S3)),
    ("aws-s3-delete-objects", ["-c", '{Objects: [.Contents[] | select(.StorageClass == "GLACIER") | {Key}], Quiet: true}'], j(S3)),
    ("aws-s3-etags", ["-r", '.Contents[] | .ETag | fromjson'], j(S3)),
    ("aws-s3-extensions", ["-c", '[.Contents[].Key | split(".") | last] | group_by(.) | map({(.[0]): length}) | add'], j(S3)),
    ("aws-s3-dirs", ["-c", '[.Contents[].Key | split("/") | .[:-1] | join("/") | select(. != "")] | unique'], j(S3)),
    ("aws-iam-actions", ["-c", '[.PolicyVersion.Document.Statement[] | select(.Effect == "Allow") | .Action | if type == "string" then [.] else . end | .[]] | unique'], j(IAM)),
    ("aws-iam-resources", ["-r", '.PolicyVersion.Document.Statement[] | .Resource | arrays[], strings'], j(IAM)),
    ("aws-iam-wildcards", ["-r", '.PolicyVersion.Document.Statement[] | select([.Action] | flatten | any(endswith("*"))) | .Sid // "(no sid)"'], j(IAM)),
    ("aws-iam-add-statement", ["-c", '.PolicyVersion.Document | .Statement += [{"Effect": "Allow", "Action": "logs:PutLogEvents", "Resource": "*"}] | .Statement | length'], j(IAM)),
    ("aws-lambda-runtimes", ["-c", '.Functions | group_by(.Runtime) | map({runtime: .[0].Runtime, functions: map(.FunctionName)})'], j(LAMBDA)),
    ("aws-lambda-deprecated", ["-r", '.Functions[] | select(.Runtime | test("^(python3\\\\.[0-9]|nodejs1[0-8])\\\\.?")) | select(.Runtime != "python3.12") | .FunctionName'], j(LAMBDA)),
    ("aws-lambda-env", ["-r", '.Functions[] | select(.Environment) | .FunctionName as $f | .Environment.Variables | to_entries[] | "\\($f): \\(.key)=\\(.value)"'], j(LAMBDA)),
    ("aws-lambda-memory-stats", ["-c", '[.Functions[].MemorySize] | {min: min, max: max, avg: (add / length)}'], j(LAMBDA)),
    ("aws-lambda-stale", ["-r", '.Functions[] | select(.LastModified[0:10] < "2026-01-01") | .FunctionName'], j(LAMBDA)),
    ("aws-cfn-outputs", ["-c", '.Stacks | map({(.StackName): ((.Outputs // []) | map({(.OutputKey): .OutputValue}) | add)}) | add'], j(CFN)),
    ("aws-cfn-failed", ["-r", '.Stacks[] | select(.StackStatus | test("FAILED|ROLLBACK")) | "\\(.StackName)\\t\\(.StackStatus)"'], j(CFN)),
    ("aws-cfn-output-split", ["-r", '.Stacks[] | select(.StackName == "network") | .Outputs[] | select(.OutputKey == "SubnetIds") | .OutputValue | split(",")[]'], j(CFN)),
    ("aws-cfn-tag-filter", ["-r", '.Stacks[] | select(any(.Tags[]?; .Key == "team" and .Value == "core")) | .StackName'], j(CFN)),
    ("aws-sts-account", ["-r", '.Account'], j(STS)),
    ("aws-sts-role", ["-r", '.Arn | split("/") | .[1]'], j(STS)),
    ("aws-sts-export", ["-r", '.Credentials | "export AWS_ACCESS_KEY_ID=\\(.AccessKeyId)\\nexport AWS_SECRET_ACCESS_KEY=\\(.SecretAccessKey | @sh)\\nexport AWS_SESSION_TOKEN=\\(.SessionToken | @sh)"'], j(CREDS)),
    ("aws-cloudwatch-sorted", ["-c", '.Datapoints | sort_by(.Timestamp) | map([.Timestamp[11:16], .Average])'], j(METRICS)),
    ("aws-cloudwatch-peak", ["-c", '.Datapoints | max_by(.Maximum) | {at: .Timestamp, max: .Maximum}'], j(METRICS)),
    ("aws-cloudwatch-mean", ["-c", '.Datapoints | map(.Average) | add / length | . * 100 | round / 100'], j(METRICS)),
    ("aws-ecs-unhealthy", ["-r", '.services[] | select(.runningCount < .desiredCount) | "\\(.serviceName): \\(.runningCount)/\\(.desiredCount)"'], j(ECS)),
    ("aws-ecs-task-revisions", ["-c", '.services | map({(.serviceName): (.deployments[] | select(.status == "PRIMARY") | .taskDefinition | split(":") | last | tonumber)}) | add'], j(ECS)),
    ("aws-ecs-rollouts", ["-r", '.services[] | select(any(.deployments[]; .rolloutState == "IN_PROGRESS")) | .serviceName'], j(ECS)),
    ("aws-route53-records", ["-r", '.ResourceRecordSets[] | "\\(.Name | rtrimstr("."))\\t\\(.Type)\\t\\(.ResourceRecords // [] | map(.Value) | join(" "))\\t\\(.AliasTarget.DNSName // "")"'], j(R53)),
    ("aws-route53-change-batch", ["-c", '{Changes: [.ResourceRecordSets[] | select(.Type == "CNAME") | {Action: "DELETE", ResourceRecordSet: .}]}'], j(R53)),
    ("aws-route53-ttl-short", ["-r", '.ResourceRecordSets[] | select(.TTL and .TTL < 300) | .Name'], j(R53)),
    ("aws-sqs-bodies", ["-c", '[.Messages[] | .Body | fromjson] | map(.total) | add'], j(SQS)),
    ("aws-sqs-retried", ["-r", '.Messages[] | select(.Attributes.ApproximateReceiveCount | tonumber > 3) | .MessageId'], j(SQS)),
    ("aws-sqs-delete-batch", ["-c", '{Entries: [.Messages[] | {Id: .MessageId, ReceiptHandle}]}'], j(SQS)),
]

# --- GitHub: gh and the REST API ----------------------------------------

PRS = [
    {"number": 101, "title": "Add retries to the uploader", "state": "OPEN", "isDraft": False, "author": {"login": "alice"},
     "labels": [{"name": "enhancement"}, {"name": "needs-review"}], "reviewDecision": "REVIEW_REQUIRED", "additions": 120, "deletions": 14,
     "createdAt": "2026-10-28T09:00:00Z", "headRefName": "uploader-retries", "baseRefName": "main", "mergeable": "MERGEABLE",
     "statusCheckRollup": [{"name": "build", "conclusion": "SUCCESS"}, {"name": "lint", "conclusion": "FAILURE"}],
     "reviewRequests": [{"login": "bob"}, {"name": "core-team"}], "files": [{"path": "src/upload.rs", "additions": 100}, {"path": "README.md", "additions": 20}]},
    {"number": 102, "title": "Bump serde to 1.0.215", "state": "OPEN", "isDraft": False, "author": {"login": "dependabot[bot]"},
     "labels": [{"name": "dependencies"}], "reviewDecision": "APPROVED", "additions": 4, "deletions": 4,
     "createdAt": "2026-11-01T06:12:00Z", "headRefName": "dependabot/cargo/serde-1.0.215", "baseRefName": "main", "mergeable": "MERGEABLE",
     "statusCheckRollup": [{"name": "build", "conclusion": "SUCCESS"}, {"name": "lint", "conclusion": "SUCCESS"}], "reviewRequests": [], "files": [{"path": "Cargo.lock", "additions": 4}]},
    {"number": 99, "title": "WIP: new parser", "state": "OPEN", "isDraft": True, "author": {"login": "carol"},
     "labels": [], "reviewDecision": None, "additions": 2300, "deletions": 800,
     "createdAt": "2026-09-15T14:30:00Z", "headRefName": "parser", "baseRefName": "main", "mergeable": "CONFLICTING",
     "statusCheckRollup": [], "reviewRequests": [], "files": [{"path": "src/parse.rs", "additions": 2300}]},
]
ISSUES = [
    {"number": 50, "title": "Crash on empty input", "state": "open", "user": {"login": "dave"}, "labels": [{"name": "bug"}, {"name": "p1"}], "assignees": [{"login": "alice"}], "comments": 7, "created_at": "2026-10-01T10:00:00Z", "pull_request": None, "milestone": {"title": "v1.0"}},
    {"number": 51, "title": "Docs: explain --json", "state": "open", "user": {"login": "erin"}, "labels": [{"name": "docs"}], "assignees": [], "comments": 0, "created_at": "2026-10-20T10:00:00Z", "milestone": None},
    {"number": 52, "title": "Fix crash on empty input", "state": "closed", "user": {"login": "alice"}, "labels": [{"name": "bug"}], "assignees": [{"login": "alice"}], "comments": 2, "created_at": "2026-10-02T10:00:00Z", "pull_request": {"url": "https://api.github.test/pulls/52"}, "milestone": {"title": "v1.0"}},
    {"number": 53, "title": "Support Windows paths", "state": "open", "user": {"login": "frank"}, "labels": [{"name": "enhancement"}, {"name": "p2"}], "assignees": [{"login": "bob"}, {"login": "carol"}], "comments": 12, "created_at": "2026-08-11T10:00:00Z"},
]
RUNS = {"total_count": 4, "workflow_runs": [
    {"id": 9001, "name": "CI", "head_branch": "main", "status": "completed", "conclusion": "success", "event": "push", "run_started_at": "2026-11-03T01:00:00Z", "updated_at": "2026-11-03T01:07:30Z", "head_sha": "a1b2c3d4e5"},
    {"id": 9002, "name": "CI", "head_branch": "uploader-retries", "status": "completed", "conclusion": "failure", "event": "pull_request", "run_started_at": "2026-11-03T02:00:00Z", "updated_at": "2026-11-03T02:03:10Z", "head_sha": "f6e5d4c3b2"},
    {"id": 9003, "name": "Release", "head_branch": "v1.4.0", "status": "in_progress", "conclusion": None, "event": "push", "run_started_at": "2026-11-03T03:00:00Z", "updated_at": "2026-11-03T03:01:00Z", "head_sha": "0099aabbcc"},
    {"id": 9004, "name": "CI", "head_branch": "main", "status": "completed", "conclusion": "cancelled", "event": "schedule", "run_started_at": "2026-11-02T00:00:00Z", "updated_at": "2026-11-02T00:00:40Z", "head_sha": "1234567890"},
]}
RELEASES = [
    {"tag_name": "v1.4.0", "name": "1.4.0", "draft": False, "prerelease": False, "published_at": "2026-11-01T12:00:00Z",
     "assets": [{"name": "tool-x86_64-unknown-linux-gnu.tar.gz", "size": 4200000, "download_count": 310, "browser_download_url": "https://github.test/releases/v1.4.0/tool-x86_64-unknown-linux-gnu.tar.gz"},
                {"name": "tool-aarch64-apple-darwin.tar.gz", "size": 3900000, "download_count": 120, "browser_download_url": "https://github.test/releases/v1.4.0/tool-aarch64-apple-darwin.tar.gz"},
                {"name": "SHA256SUMS", "size": 300, "download_count": 50, "browser_download_url": "https://github.test/releases/v1.4.0/SHA256SUMS"}]},
    {"tag_name": "v1.5.0-rc.1", "name": "1.5.0 RC 1", "draft": False, "prerelease": True, "published_at": "2026-11-02T12:00:00Z", "assets": []},
    {"tag_name": "v1.3.2", "name": "1.3.2", "draft": False, "prerelease": False, "published_at": "2026-09-01T12:00:00Z",
     "assets": [{"name": "tool-x86_64-unknown-linux-gnu.tar.gz", "size": 4100000, "download_count": 2050, "browser_download_url": "https://github.test/releases/v1.3.2/tool-x86_64-unknown-linux-gnu.tar.gz"}]},
]
COMMITS = [
    {"sha": "a1b2c3d4e5f6", "commit": {"message": "fix: handle empty input\n\nCloses #50", "author": {"name": "Alice", "email": "alice@acme.test", "date": "2026-11-02T10:00:00Z"}}, "author": {"login": "alice"}, "parents": [{"sha": "p1"}]},
    {"sha": "b2c3d4e5f6a1", "commit": {"message": "feat(upload): retries", "author": {"name": "Alice", "email": "alice@acme.test", "date": "2026-11-01T10:00:00Z"}}, "author": {"login": "alice"}, "parents": [{"sha": "p2"}]},
    {"sha": "c3d4e5f6a1b2", "commit": {"message": "Merge pull request #48 from bob/docs", "author": {"name": "Bob", "email": "bob@acme.test", "date": "2026-10-31T10:00:00Z"}}, "author": {"login": "bob"}, "parents": [{"sha": "p3"}, {"sha": "p4"}]},
    {"sha": "d4e5f6a1b2c3", "commit": {"message": "chore: bump deps", "author": {"name": "dependabot[bot]", "email": "bot@github.test", "date": "2026-10-30T10:00:00Z"}}, "author": None, "parents": [{"sha": "p5"}]},
]

P, I = j(PRS), j(ISSUES)
CASES += [
    ("gh-pr-list", ["-r", '.[] | "#\\(.number)\\t\\(.title)\\t\\(.author.login)"'], P),
    ("gh-pr-ready", ["-r", '.[] | select(.isDraft | not) | select(.reviewDecision == "APPROVED") | .number'], P),
    ("gh-pr-failing-checks", ["-c", '[.[] | {number, failing: [.statusCheckRollup[] | select(.conclusion != "SUCCESS") | .name]} | select(.failing | length > 0)]'], P),
    ("gh-pr-size", ["-c", 'map({number, size: (.additions + .deletions), label: (if .additions + .deletions > 1000 then "XL" elif .additions + .deletions > 100 then "M" else "S" end)})'], P),
    ("gh-pr-bots", ["-r", '.[] | select(.author.login | endswith("[bot]")) | .headRefName'], P),
    ("gh-pr-reviewers", ["-c", '[.[].reviewRequests[] | .login // .name] | unique'], P),
    ("gh-pr-labels", ["-c", 'map({(.number | tostring): [.labels[].name]}) | add'], P),
    ("gh-pr-conflicts", ["-r", '.[] | select(.mergeable == "CONFLICTING") | "\\(.number) \\(.headRefName)"'], P),
    ("gh-pr-touched-files", ["-r", '[.[].files[].path] | unique[]'], P),
    ("gh-pr-oldest-open", ["-r", 'map(select(.state == "OPEN")) | sort_by(.createdAt) | first | "#\\(.number) since \\(.createdAt[:10])"'], P),
    ("gh-pr-markdown", ["-r", '.[] | "- [\\(if .isDraft then " " else "x" end)] #\\(.number) \\(.title)"'], P),
    ("gh-pr-stats", ["-c", '{open: length, drafts: map(select(.isDraft)) | length, lines: (map(.additions + .deletions) | add)}'], P),
    ("gh-issues-not-prs", ["-r", '.[] | select(.pull_request == null) | .number'], I),
    ("gh-issues-by-label", ["-c", '[.[] | .labels[].name] | group_by(.) | map({label: .[0], count: length}) | sort_by(-.count, .label)'], I),
    ("gh-issues-unassigned", ["-r", '.[] | select(.state == "open" and (.assignees | length) == 0) | .title'], I),
    ("gh-issues-priority", ["-c", 'map(. + {priority: ([.labels[].name | select(startswith("p")) | ltrimstr("p") | tonumber] | min // 9)}) | sort_by(.priority) | map(.number)'], I),
    ("gh-issues-milestone", ["-c", 'group_by(.milestone.title // "none") | map({(.[0].milestone.title // "none"): map(.number)}) | add'], I),
    ("gh-issues-hot", ["-r", 'max_by(.comments) | "\\(.number): \\(.comments) comments"'], I),
    ("gh-issues-assignee-load", ["-c", 'reduce (.[] | select(.state == "open") | .assignees[].login) as $a ({}; .[$a] += 1)'], I),
    ("gh-issues-csv", ["-r", '.[] | [.number, .state, .title, ([.labels[].name] | join(";"))] | @csv'], I),
    ("gh-runs-failed", ["-r", '.workflow_runs[] | select(.conclusion == "failure") | "\\(.name) on \\(.head_branch): \\(.head_sha[:7])"'], j(RUNS)),
    ("gh-runs-duration", ["-c", '.workflow_runs | map(select(.status == "completed") | {id, seconds: ((.updated_at | fromdate) - (.run_started_at | fromdate))})'], j(RUNS)),
    ("gh-runs-latest-main", ["-c", '[.workflow_runs[] | select(.head_branch == "main")] | sort_by(.run_started_at) | last | {id, conclusion}'], j(RUNS)),
    ("gh-runs-summary", ["-c", '.workflow_runs | group_by(.conclusion // "running") | map({key: (.[0].conclusion // "running"), value: length}) | from_entries'], j(RUNS)),
    ("gh-runs-cancel-ids", ["-r", '.workflow_runs[] | select(.status == "in_progress") | .id'], j(RUNS)),
    ("gh-release-latest", ["-r", 'map(select(.prerelease | not)) | sort_by(.published_at) | last | .tag_name'], j(RELEASES)),
    ("gh-release-asset-url", ["-r", '.[0].assets[] | select(.name | test("x86_64.*linux")) | .browser_download_url'], j(RELEASES)),
    ("gh-release-downloads", ["-c", 'map({tag: .tag_name, downloads: ([.assets[].download_count] | add // 0)})'], j(RELEASES)),
    ("gh-release-semver-sort", ["-r", 'map(.tag_name | ltrimstr("v") | split("-")[0] | split(".") | map(tonumber)) | sort | reverse | map(join(".")) | .[]'], j(RELEASES)),
    ("gh-release-sizes-mb", ["-c", '[.[].assets[] | {name, mb: (.size / 1000000 * 10 | round / 10)}] | unique_by(.name)'], j(RELEASES)),
    ("gh-commits-oneline", ["-r", '.[] | "\\(.sha[:7]) \\(.commit.message | split("\\n")[0])"'], j(COMMITS)),
    ("gh-commits-conventional", ["-c", '[.[].commit.message | split("\\n")[0] | capture("^(?<type>[a-z]+)(\\\\((?<scope>[^)]+)\\\\))?: ") | .type] | group_by(.) | map({(.[0]): length}) | add'], j(COMMITS)),
    ("gh-commits-merges", ["-r", '.[] | select(.parents | length > 1) | .sha[:7]'], j(COMMITS)),
    ("gh-commits-authors", ["-c", 'map(.author.login // .commit.author.name) | unique'], j(COMMITS)),
    ("gh-commits-closes", ["-c", '[.[].commit.message | scan("(?i)closes #([0-9]+)")[] | tonumber]'], j(COMMITS)),
    ("gh-commits-by-day", ["-c", 'group_by(.commit.author.date[:10]) | map({day: .[0].commit.author.date[:10], n: length})'], j(COMMITS)),
    ("gh-api-paginate-slurp", ["-s", "-c", 'add | length'], lines([[{"id": 1}, {"id": 2}], [{"id": 3}]])),
    ("gh-api-link-header", ["-r", 'split(", ") | map(capture("<(?<url>[^>]+)>; rel=\\"(?<rel>[a-z]+)\\"")) | map({(.rel): .url}) | add | .next'],
     j('<https://api.github.test/repos/x/y/issues?page=2>; rel="next", <https://api.github.test/repos/x/y/issues?page=9>; rel="last"')),
    ("gh-graphql-nodes", ["-c", '.data.repository.issues.nodes | map({n: .number, t: .title})'], j({"data": {"repository": {"issues": {"nodes": [{"number": 1, "title": "a"}, {"number": 2, "title": "b"}], "pageInfo": {"hasNextPage": False}}}}})),
    ("gh-graphql-payload", ["-n", "-c", '--arg', 'owner', 'acme', '--arg', 'repo', 'tool', '{query: "query($o: String!, $r: String!) { repository(owner: $o, name: $r) { id } }", variables: {o: $owner, r: $repo}}'], ""),
]

# --- Docker --------------------------------------------------------------

INSPECT = [{
    "Id": "f00dcafe1234", "Name": "/db", "Created": "2026-11-01T10:00:00.123456789Z",
    "State": {"Status": "running", "Running": True, "Pid": 4242, "ExitCode": 0, "StartedAt": "2026-11-01T10:00:01Z", "Health": {"Status": "healthy", "FailingStreak": 0, "Log": [{"ExitCode": 0, "Output": "ok"}]}},
    "Config": {"Image": "postgres:16", "Env": ["POSTGRES_USER=app", "POSTGRES_PASSWORD=s3cret", "PGDATA=/var/lib/postgresql/data", "TZ=Europe/Paris=x"],
               "Labels": {"com.docker.compose.project": "shop", "com.docker.compose.service": "db"}, "ExposedPorts": {"5432/tcp": {}}, "Cmd": ["postgres", "-c", "max_connections=200"]},
    "HostConfig": {"RestartPolicy": {"Name": "unless-stopped"}, "Memory": 1073741824, "PortBindings": {"5432/tcp": [{"HostIp": "127.0.0.1", "HostPort": "5432"}]}},
    "Mounts": [{"Type": "volume", "Name": "shop_pg", "Source": "/var/lib/docker/volumes/shop_pg/_data", "Destination": "/var/lib/postgresql/data", "RW": True},
               {"Type": "bind", "Source": "/srv/init", "Destination": "/docker-entrypoint-initdb.d", "RW": False}],
    "NetworkSettings": {"Ports": {"5432/tcp": [{"HostIp": "127.0.0.1", "HostPort": "5432"}]}, "Networks": {"shop_default": {"IPAddress": "172.18.0.2", "Gateway": "172.18.0.1", "Aliases": ["db", "f00dcafe1234"]}}},
}]
PS = [
    {"ID": "f00dcafe1234", "Image": "postgres:16", "Names": "shop-db-1", "State": "running", "Status": "Up 2 days (healthy)", "Ports": "127.0.0.1:5432->5432/tcp", "Labels": "com.docker.compose.project=shop,com.docker.compose.service=db", "CreatedAt": "2026-11-01 10:00:00 +0000 UTC"},
    {"ID": "beef00112233", "Image": "ghcr.io/acme/shop:2.1", "Names": "shop-web-1", "State": "running", "Status": "Up 2 days", "Ports": "0.0.0.0:8080->8080/tcp, :::8080->8080/tcp", "Labels": "com.docker.compose.project=shop,com.docker.compose.service=web", "CreatedAt": "2026-11-01 10:00:05 +0000 UTC"},
    {"ID": "dead44556677", "Image": "redis:7", "Names": "cache", "State": "exited", "Status": "Exited (137) 3 hours ago", "Ports": "", "Labels": "", "CreatedAt": "2026-10-20 08:00:00 +0000 UTC"},
]
IMAGES = [
    {"Repository": "postgres", "Tag": "16", "ID": "sha256:aaa", "Size": "432MB", "CreatedSince": "3 weeks ago"},
    {"Repository": "ghcr.io/acme/shop", "Tag": "2.1", "ID": "sha256:bbb", "Size": "98.5MB", "CreatedSince": "2 days ago"},
    {"Repository": "<none>", "Tag": "<none>", "ID": "sha256:ccc", "Size": "1.2GB", "CreatedSince": "2 months ago"},
    {"Repository": "redis", "Tag": "7", "ID": "sha256:ddd", "Size": "117MB", "CreatedSince": "5 weeks ago"},
]
COMPOSE = {"name": "shop", "services": {
    "web": {"image": "ghcr.io/acme/shop:2.1", "ports": [{"target": 8080, "published": "8080"}], "depends_on": {"db": {"condition": "service_healthy"}, "cache": {"condition": "service_started"}}, "environment": {"DATABASE_URL": "postgres://app@db/app", "LOG": None}},
    "db": {"image": "postgres:16", "volumes": [{"type": "volume", "source": "pg", "target": "/var/lib/postgresql/data"}], "healthcheck": {"test": ["CMD", "pg_isready"], "interval": "10s"}},
    "cache": {"image": "redis:7", "profiles": ["full"]}}, "volumes": {"pg": {}}}

D = j(INSPECT)
CASES += [
    ("docker-inspect-ip", ["-r", '.[0].NetworkSettings.Networks | to_entries[] | "\\(.key) \\(.value.IPAddress)"'], D),
    ("docker-inspect-env-object", ["-c", '.[0].Config.Env | map(capture("^(?<k>[^=]+)=(?<v>.*)$")) | map({(.k): .v}) | add'], D),
    ("docker-inspect-env-redact", ["-r", '.[0].Config.Env[] | sub("(?<k>PASSWORD)=.*"; "\\(.k)=***")'], D),
    ("docker-inspect-ports", ["-r", '.[0].NetworkSettings.Ports | to_entries[] | .key as $c | .value[] | "\\(.HostIp):\\(.HostPort) -> \\($c)"'], D),
    ("docker-inspect-mounts", ["-c", '.[0].Mounts | map(select(.Type == "bind")) | map({src: .Source, dst: .Destination, ro: (.RW | not)})'], D),
    ("docker-inspect-health", ["-r", '.[0].State.Health.Status // "none"'], D),
    ("docker-inspect-labels", ["-r", '.[0].Config.Labels["com.docker.compose.service"]'], D),
    ("docker-inspect-cmd", ["-r", '.[0].Config.Cmd | @sh'], D),
    ("docker-inspect-memory-mb", ["-c", '.[0].HostConfig.Memory / 1048576'], D),
    ("docker-inspect-created-date", ["-r", '.[0].Created | sub("\\\\.[0-9]+Z$"; "Z") | fromdate | strftime("%Y-%m-%d %H:%M")'], D),
    ("docker-ps-lines", ["-r", 'select(.State == "running") | .Names'], lines(PS)),
    ("docker-ps-slurp-table", ["-s", "-r", '.[] | [.ID[:12], .Image, .State] | @tsv'], lines(PS)),
    ("docker-ps-exit-codes", ["-r", 'select(.State == "exited") | .Status | capture("Exited \\\\((?<code>[0-9]+)\\\\)").code'], lines(PS)),
    ("docker-ps-labels-split", ["-c", 'select(.Labels != "") | .Labels | split(",") | map(split("=") | {(.[0]): .[1]}) | add'], lines(PS)),
    ("docker-ps-published-ports", ["-c", '[inputs | .Ports | split(", ")[] | select(. != "") | capture(":(?<host>[0-9]+)->(?<ctr>[0-9]+)") | .host] | unique', "-n"], lines(PS)),
    ("docker-ps-compose-projects", ["-s", "-c", 'map(.Labels | capture("com.docker.compose.project=(?<p>[^,]+)")?.p) | unique'], lines(PS)),
    ("docker-images-dangling", ["-r", 'select(.Repository == "<none>") | .ID'], lines(IMAGES)),
    ("docker-images-size-mb", ["-s", "-c", 'map(.Size | if endswith("GB") then (rtrimstr("GB") | tonumber * 1000) else (rtrimstr("MB") | tonumber) end) | add'], lines(IMAGES)),
    ("docker-images-refs", ["-r", 'select(.Tag != "<none>") | "\\(.Repository):\\(.Tag)"'], lines(IMAGES)),
    ("docker-compose-images", ["-r", '.services | to_entries[] | "\\(.key)=\\(.value.image)"'], j(COMPOSE)),
    ("docker-compose-deps", ["-c", '.services | map_values(.depends_on // {} | keys)'], j(COMPOSE)),
    ("docker-compose-healthy-deps", ["-c", '[.services[] | .depends_on // {} | to_entries[] | select(.value.condition == "service_healthy") | .key] | unique'], j(COMPOSE)),
    ("docker-compose-profiles", ["-r", '.services | to_entries[] | select(.value.profiles) | .key'], j(COMPOSE)),
    ("docker-compose-env-unset", ["-r", '.services[].environment // {} | to_entries[] | select(.value == null) | .key'], j(COMPOSE)),
    ("docker-compose-volumes", ["-c", '[.services[].volumes[]? | select(.type == "volume") | .source] - (.volumes | keys)'], j(COMPOSE)),
    ("docker-daemon-json-merge", ["-c", '. * {"log-driver": "json-file", "log-opts": {"max-size": "10m"}}'], j({"log-opts": {"max-file": "3"}, "features": {"buildkit": True}})),
]

# --- Terraform and npm ---------------------------------------------------

TFSTATE = {"version": 4, "terraform_version": "1.9.8", "resources": [
    {"mode": "managed", "type": "aws_instance", "name": "web", "provider": "provider[\"registry.terraform.io/hashicorp/aws\"]", "instances": [{"index_key": 0, "attributes": {"id": "i-1", "instance_type": "t3.small", "tags": {"Name": "web-0"}}}, {"index_key": 1, "attributes": {"id": "i-2", "instance_type": "t3.small", "tags": {"Name": "web-1"}}}]},
    {"mode": "managed", "type": "aws_s3_bucket", "name": "assets", "provider": "provider[\"registry.terraform.io/hashicorp/aws\"]", "instances": [{"attributes": {"id": "acme-assets", "bucket": "acme-assets", "tags": None}}]},
    {"mode": "data", "type": "aws_ami", "name": "ubuntu", "provider": "provider[\"registry.terraform.io/hashicorp/aws\"]", "instances": [{"attributes": {"id": "ami-123"}}]},
    {"module": "module.vpc", "mode": "managed", "type": "aws_vpc", "name": "this", "provider": "provider[\"registry.terraform.io/hashicorp/aws\"]", "instances": [{"attributes": {"id": "vpc-9", "cidr_block": "10.0.0.0/16"}}]},
], "outputs": {"web_ips": {"value": ["10.0.1.10", "10.0.1.11"], "type": ["list", "string"]}, "db_password": {"value": "hunter2", "type": "string", "sensitive": True}}}
TFPLAN = {"format_version": "1.2", "resource_changes": [
    {"address": "aws_instance.web[1]", "type": "aws_instance", "change": {"actions": ["delete", "create"], "before": {"instance_type": "t3.small"}, "after": {"instance_type": "t3.medium"}}},
    {"address": "aws_s3_bucket.assets", "type": "aws_s3_bucket", "change": {"actions": ["no-op"]}},
    {"address": "aws_iam_role.ci", "type": "aws_iam_role", "change": {"actions": ["create"], "before": None, "after": {"name": "ci"}}},
    {"address": "module.vpc.aws_vpc.this", "type": "aws_vpc", "change": {"actions": ["update"], "before": {"tags": {}}, "after": {"tags": {"env": "prod"}}}},
]}
PKG = {"name": "@acme/shop", "version": "2.1.0", "private": True, "type": "module", "engines": {"node": ">=20"},
       "scripts": {"build": "vite build", "dev": "vite", "test": "vitest run", "lint": "eslint .", "prepare": "husky"},
       "dependencies": {"react": "^19.0.0", "react-dom": "^19.0.0", "zod": "^3.23.8", "lodash": "4.17.21"},
       "devDependencies": {"vite": "^6.0.1", "vitest": "^2.1.5", "typescript": "~5.6.3", "eslint": "^9.15.0"},
       "workspaces": ["packages/*"]}
LOCK = {"name": "@acme/shop", "lockfileVersion": 3, "packages": {
    "": {"name": "@acme/shop", "version": "2.1.0"},
    "node_modules/react": {"version": "19.0.0", "license": "MIT", "resolved": "https://registry.npmjs.test/react/-/react-19.0.0.tgz"},
    "node_modules/lodash": {"version": "4.17.21", "license": "MIT"},
    "node_modules/zod": {"version": "3.23.8", "license": "MIT"},
    "node_modules/vite": {"version": "6.0.1", "license": "MIT", "dev": True},
    "node_modules/vite/node_modules/esbuild": {"version": "0.24.0", "license": "MIT", "dev": True, "optional": True},
    "node_modules/left-pad": {"version": "1.3.0", "license": "WTFPL"}}}
AUDIT = {"vulnerabilities": {"lodash": {"severity": "high", "via": [{"title": "Prototype Pollution", "url": "https://github.test/advisories/1"}], "fixAvailable": True},
                             "esbuild": {"severity": "moderate", "via": ["vite"], "fixAvailable": {"name": "vite", "version": "6.0.5"}}},
         "metadata": {"vulnerabilities": {"info": 0, "low": 0, "moderate": 1, "high": 1, "critical": 0, "total": 2}}}

CASES += [
    ("tf-state-resources", ["-r", '.resources[] | select(.mode == "managed") | [.module, .type, .name] | map(select(. != null)) | join(".")'], j(TFSTATE)),
    ("tf-state-instances", ["-c", '[.resources[] | select(.type == "aws_instance") | .instances[] | {key: .index_key, id: .attributes.id}]'], j(TFSTATE)),
    ("tf-state-outputs", ["-c", '.outputs | with_entries(select(.value.sensitive | not)) | map_values(.value)'], j(TFSTATE)),
    ("tf-state-providers", ["-r", '[.resources[].provider | capture("\\"(?<p>[^\\"]+)\\"").p] | unique[]'], j(TFSTATE)),
    ("tf-state-count-by-type", ["-c", '[.resources[] | {type, n: (.instances | length)}] | group_by(.type) | map({(.[0].type): (map(.n) | add)}) | add'], j(TFSTATE)),
    ("tf-state-untagged", ["-r", '.resources[] | select(.mode == "managed") | .instances[] | select(.attributes | has("tags") and .tags == null) | .attributes.id'], j(TFSTATE)),
    ("tf-plan-changes", ["-r", '.resource_changes[] | select(.change.actions != ["no-op"]) | "\\(.change.actions | join("/"))\\t\\(.address)"'], j(TFPLAN)),
    ("tf-plan-destroys", ["-e", 'any(.resource_changes[]; .change.actions | index("delete"))'], j(TFPLAN)),
    ("tf-plan-summary", ["-c", '[.resource_changes[].change.actions | join("/")] | group_by(.) | map({(.[0]): length}) | add'], j(TFPLAN)),
    ("tf-plan-diff", ["-c", '.resource_changes[] | select(.change.actions == ["update"]) | {address, before: .change.before, after: .change.after}'], j(TFPLAN)),
    ("tf-vars-json", ["-n", "-c", "--arg", "env", "prod", "--argjson", "count", "3", '{env: $env, instance_count: $count, tags: {managed_by: "terraform", env: $env}}'], ""),
    ("npm-scripts", ["-r", '.scripts | keys[]'], j(PKG)),
    ("npm-deps-flat", ["-r", '(.dependencies + .devDependencies) | to_entries[] | "\\(.key)@\\(.value)"'], j(PKG)),
    ("npm-pinned", ["-r", '.dependencies | to_entries[] | select(.value | test("^[0-9]")) | .key'], j(PKG)),
    ("npm-bump-patch", ["-r", '.version | split(".") | map(tonumber) | .[2] += 1 | map(tostring) | join(".")'], j(PKG)),
    ("npm-set-version", ["--indent", "2", '.version = "2.2.0"'], j(PKG)),
    ("npm-add-script", ["-c", '.scripts += {typecheck: "tsc --noEmit"} | .scripts | keys'], j(PKG)),
    ("npm-remove-dep", ["-c", 'del(.dependencies.lodash) | .dependencies | keys'], j(PKG)),
    ("npm-has-react", ["-e", '.dependencies | has("react")'], j(PKG)),
    ("npm-caret-majors", ["-c", '.dependencies | map_values(ltrimstr("^") | ltrimstr("~") | split(".")[0] | tonumber)'], j(PKG)),
    ("npm-engines", ["-r", '.engines.node // "any"'], j(PKG)),
    ("npm-lock-versions", ["-r", '.packages | to_entries[] | select(.key != "") | "\\(.key | sub("^.*node_modules/"; ""))@\\(.value.version)"'], j(LOCK)),
    ("npm-lock-licenses", ["-c", '[.packages[] | .license // empty] | group_by(.) | map({(.[0]): length}) | add'], j(LOCK)),
    ("npm-lock-prod-only", ["-r", '.packages | to_entries[] | select(.key != "" and (.value.dev | not)) | .key | ltrimstr("node_modules/")'], j(LOCK)),
    ("npm-lock-nested", ["-r", '.packages | keys[] | select(test("node_modules/.*/node_modules/"))'], j(LOCK)),
    ("npm-audit-high", ["-r", '.vulnerabilities | to_entries[] | select(.value.severity == "high" or .value.severity == "critical") | .key'], j(AUDIT)),
    ("npm-audit-summary", ["-r", '.metadata.vulnerabilities | to_entries | map(select(.value > 0) | "\\(.value) \\(.key)") | join(", ")'], j(AUDIT)),
    ("npm-audit-fixes", ["-c", '.vulnerabilities | map_values(.fixAvailable | if type == "object" then "\\(.name)@\\(.version)" else . end)'], j(AUDIT)),
]

# --- Logs (JSON Lines) ---------------------------------------------------

APP_LOG = lines([
    {"ts": "2026-11-03T01:00:00.120Z", "level": "info", "msg": "server started", "port": 8080},
    {"ts": "2026-11-03T01:00:05.002Z", "level": "info", "msg": "request", "method": "GET", "path": "/api/items", "status": 200, "ms": 12, "user": "u1"},
    {"ts": "2026-11-03T01:00:05.900Z", "level": "warn", "msg": "slow request", "method": "POST", "path": "/api/orders", "status": 201, "ms": 1450, "user": "u2"},
    {"ts": "2026-11-03T01:00:06.311Z", "level": "error", "msg": "db timeout", "method": "GET", "path": "/api/orders", "status": 500, "ms": 5003, "err": {"code": "ETIMEDOUT", "stack": "Error: timeout\n    at db.js:10"}},
    {"ts": "2026-11-03T01:00:07.000Z", "level": "info", "msg": "request", "method": "GET", "path": "/api/items", "status": 200, "ms": 9, "user": "u1"},
    {"ts": "2026-11-03T01:00:08.500Z", "level": "error", "msg": "db timeout", "method": "GET", "path": "/api/items", "status": 503, "ms": 5001},
    {"ts": "2026-11-03T01:01:00.000Z", "level": "debug", "msg": "gc", "heap_mb": 211.5},
])
ACCESS_LOG = "".join([
    '{"remote_addr":"203.0.113.9","time":"03/Nov/2026:01:00:05 +0000","request":"GET /index.html HTTP/1.1","status":"200","body_bytes_sent":"5120","http_user_agent":"curl/8.5.0"}\n',
    '{"remote_addr":"198.51.100.4","time":"03/Nov/2026:01:00:06 +0000","request":"GET /admin HTTP/1.1","status":"403","body_bytes_sent":"162","http_user_agent":"Mozilla/5.0"}\n',
    '{"remote_addr":"203.0.113.9","time":"03/Nov/2026:01:00:07 +0000","request":"POST /api/login HTTP/1.1","status":"401","body_bytes_sent":"20","http_user_agent":"curl/8.5.0"}\n',
    '{"remote_addr":"203.0.113.9","time":"03/Nov/2026:01:00:09 +0000","request":"POST /api/login HTTP/1.1","status":"200","body_bytes_sent":"310","http_user_agent":"curl/8.5.0"}\n',
])
CASES += [
    ("log-errors-only", ["-c", 'select(.level == "error") | {ts, msg, path}'], APP_LOG),
    ("log-errors-raw", ["-r", 'select(.level == "error") | "\\(.ts) \\(.msg) (\\(.err.code // "no code"))"'], APP_LOG),
    ("log-levels-count", ["-n", "-c", 'reduce inputs as $l ({}; .[$l.level] += 1)'], APP_LOG),
    ("log-slow-requests", ["-r", 'select(.ms? > 1000) | "\\(.method) \\(.path) \\(.ms)ms"'], APP_LOG),
    ("log-p50-ms", ["-s", "-c", 'map(.ms // empty) | sort | .[length / 2 | floor]'], APP_LOG),
    ("log-status-classes", ["-s", "-c", 'map(.status // empty | tostring | .[0:1] + "xx") | group_by(.) | map({(.[0]): length}) | add'], APP_LOG),
    ("log-paths-latency", ["-s", "-c", 'map(select(.path)) | group_by(.path) | map({path: .[0].path, n: length, avg_ms: (map(.ms) | add / length)})'], APP_LOG),
    ("log-users", ["-n", "-r", '[inputs | .user // empty] | unique | .[]'], APP_LOG),
    ("log-stack-first-line", ["-r", 'select(.err.stack) | .err.stack | split("\\n")[0]'], APP_LOG),
    ("log-epoch", ["-c", 'select(.level == "error") | .ts | sub("\\\\.[0-9]+Z$"; "Z") | fromdate'], APP_LOG),
    ("log-window", ["-c", 'select(.ts >= "2026-11-03T01:00:06" and .ts < "2026-11-03T01:00:08") | .msg'], APP_LOG),
    ("log-to-csv", ["-r", 'select(.status) | [.ts, .method, .path, .status, .ms] | @csv'], APP_LOG),
    ("log-keys-union", ["-s", "-c", 'map(keys) | add | unique'], APP_LOG),
    ("log-debug-dropped", ["-c", 'select(.level != "debug") | del(.err.stack)'], APP_LOG),
    ("log-first-error", ["-n", "-c", 'first(inputs | select(.level == "error")) | .ts'], APP_LOG),
    ("log-nginx-status", ["-r", '.status + " " + (.request | split(" ")[1])'], ACCESS_LOG),
    ("log-nginx-ips", ["-s", "-c", 'group_by(.remote_addr) | map({ip: .[0].remote_addr, hits: length}) | sort_by(-.hits)'], ACCESS_LOG),
    ("log-nginx-failed-logins", ["-r", 'select(.request | startswith("POST /api/login")) | select(.status | tonumber >= 400) | .remote_addr'], ACCESS_LOG),
    ("log-nginx-bytes", ["-n", '[inputs | .body_bytes_sent | tonumber] | add'], ACCESS_LOG),
    ("log-nginx-time-parse", ["-r", '.time | strptime("%d/%b/%Y:%H:%M:%S %z") | mktime | todate'], ACCESS_LOG),
]

# --- CSV and TSV out, and in with -R -------------------------------------

USERS = [{"id": 1, "name": "Ana Lima", "email": "ana@acme.test", "roles": ["admin", "dev"], "active": True, "last_login": "2026-11-02T08:00:00Z"},
         {"id": 2, "name": "Bao, Tran", "email": "bao@acme.test", "roles": ["dev"], "active": False, "last_login": None},
         {"id": 3, "name": "Chloé \"CJ\" Jones", "email": "cj@acme.test", "roles": [], "active": True, "last_login": "2026-10-30T19:45:00Z"}]
U = j(USERS)
CASES += [
    ("csv-header-rows", ["-r", '(.[0] | keys_unsorted | map(select(. != "roles"))) as $k | ($k | @csv), (.[] | [.[$k[]]] | @csv)'], U),
    ("csv-quoted-names", ["-r", '.[] | [.id, .name] | @csv'], U),
    ("tsv-with-roles", ["-r", '.[] | [.email, (.roles | join(","))] | @tsv'], U),
    ("csv-nulls", ["-r", '.[] | [.id, .last_login] | @csv'], U),
    ("csv-booleans", ["-r", '.[] | [.id, .active] | @csv'], U),
    ("csv-in-raw-lines", ["-R", "-c", 'split(",")'], "id,name,score\n1,ana,9.5\n2,bao,7\n"),
    ("csv-in-to-objects", ["-R", "-s", "-c", 'split("\\n") | map(select(length > 0) | split(",")) | .[0] as $h | .[1:] | map([$h, .] | transpose | map({(.[0]): .[1]}) | add)'], "id,name,score\n1,ana,9.5\n2,bao,7\n"),
    ("tsv-in-sum", ["-R", "-n", '[inputs | split("\\t")[1] | tonumber] | add'], "a\t1\nb\t2.5\nc\t-1\n"),
    ("csv-from-env-like", ["-R", "-c", 'capture("^(?<key>[A-Z_]+)=(?<value>.*)$")'], "HOME=/home/ana\nPATH=/usr/bin:/bin\nEMPTY=\n"),
]

# --- What people ask most (Stack Overflow's jq questions) -----------------

NESTED = {"id": 7, "meta": {"tags": ["a", "b"], "owner": {"name": "ana", "email": None}}, "items": [{"sku": "x1", "qty": 2, "price": 3.5}, {"sku": "y2", "qty": 1, "price": 10}, {"sku": "x1", "qty": 3, "price": 3.5}], "notes": ""}
N = j(NESTED)
KV = j([{"name": "region", "value": "eu-west-1"}, {"name": "env", "value": "prod"}, {"name": "env", "value": "staging"}])
PEOPLE = j([{"name": "ana", "age": 31, "city": "Lisbon", "team": "core"}, {"name": "bao", "age": 25, "city": "Hanoi", "team": "web"}, {"name": "cj", "age": 41, "city": "Lisbon", "team": "core"}, {"name": "dee", "age": 25, "city": None, "team": "web"}])
CASES += [
    ("so-get-key-with-dash", ["-r", '.["content-type"], ."x-request-id"'], j({"content-type": "application/json", "x-request-id": "r-1"})),
    ("so-key-from-variable", ["-r", "--arg", "k", "owner", '.meta[$k].name'], N),
    ("so-nested-path-from-string", ["-c", "--arg", "p", "meta.owner.name", 'getpath($p | split("."))'], N),
    ("so-select-by-value", ["-c", '.items[] | select(.sku == "x1")'], N),
    ("so-select-not-null", ["-c", '[.meta.owner | to_entries[] | select(.value != null) | .key]'], N),
    ("so-filter-empty-strings", ["-c", 'with_entries(select(.value != ""))'], N),
    ("so-remove-nulls-deep", ["-c", 'walk(if type == "object" then with_entries(select(.value != null)) else . end)'], N),
    ("so-count-items", ["-c", '.items | length'], N),
    ("so-sum-field", ["-c", '[.items[] | .qty * .price] | add'], N),
    ("so-sum-by-key", ["-c", '.items | group_by(.sku) | map({sku: .[0].sku, qty: (map(.qty) | add)})'], N),
    ("so-unique-by", ["-c", '.items | unique_by(.sku) | map(.sku)'], N),
    ("so-sort-desc", ["-c", '.items | sort_by(.price) | reverse | map(.sku)'], N),
    ("so-sort-multiple-keys", ["-c", 'sort_by(.age, .name) | map(.name)'], PEOPLE),
    ("so-group-to-object", ["-c", 'group_by(.team) | map({(.[0].team): map(.name)}) | add'], PEOPLE),
    ("so-index-by", ["-c", 'INDEX(.name) | map_values(.age)'], PEOPLE),
    ("so-max-by-age", ["-r", 'max_by(.age).name'], PEOPLE),
    ("so-min-age-ties", ["-c", '(map(.age) | min) as $m | map(select(.age == $m) | .name)'], PEOPLE),
    ("so-average", ["-c", 'map(.age) | add / length'], PEOPLE),
    ("so-null-coalesce", ["-r", '.[] | "\\(.name): \\(.city // "unknown")"'], PEOPLE),
    ("so-array-of-names-to-string", ["-r", 'map(.name) | join(", ")'], PEOPLE),
    ("so-name-value-to-object", ["-c", 'map({(.name): .value}) | add'], KV),
    ("so-name-value-from-entries", ["-c", 'map({key: .name, value}) | from_entries'], KV),
    ("so-name-value-duplicates", ["-c", 'reduce .[] as $e ({}; .[$e.name] += [$e.value])'], KV),
    ("so-object-to-array", ["-c", 'to_entries | map({k: .key, v: .value})'], j({"a": 1, "b": 2})),
    ("so-rename-key", ["-c", 'with_entries(if .key == "a" then .key = "alpha" else . end)'], j({"a": 1, "b": 2})),
    ("so-rename-keys-upcase", ["-c", 'with_entries(.key |= ascii_upcase)'], j({"a": 1, "b": 2})),
    ("so-merge-objects", ["-c", '.[0] * .[1]'], j([{"a": {"x": 1, "y": 2}, "b": 1}, {"a": {"y": 3, "z": 4}, "c": 2}])),
    ("so-merge-array-of-objects", ["-c", 'add'], j([{"a": 1}, {"b": 2}, {"a": 3}])),
    ("so-merge-two-files-slurp", ["-s", "-c", '.[0] * .[1]'], lines([{"db": {"host": "localhost", "port": 5432}}, {"db": {"host": "db.prod"}, "debug": False}])),
    ("so-add-field", ["-c", 'map(. + {seen: true})'], j([{"id": 1}, {"id": 2}])),
    ("so-add-field-computed", ["-c", '.items |= map(.total = .qty * .price)'], N),
    ("so-update-in-array", ["-c", '(.items[] | select(.sku == "y2") | .qty) |= . + 10 | .items | map(.qty)'], N),
    ("so-update-all-strings", ["-c", '(.. | strings) |= ascii_upcase'], j({"a": "x", "b": ["y", 1, {"c": "z"}]})),
    ("so-delete-key-everywhere", ["-c", 'del(.. | .secret?)'], j({"secret": 1, "a": {"secret": 2, "b": [{"secret": 3, "c": 4}]}})),
    ("so-delete-array-element", ["-c", 'del(.[] | select(. == 2))'], j([1, 2, 3, 2])),
    ("so-delete-by-index", ["-c", 'del(.[1, 3])'], j(["a", "b", "c", "d", "e"])),
    ("so-flatten-object-paths", ["-c", '[paths(scalars) as $p | {key: ($p | map(tostring) | join(".")), value: getpath($p)}] | from_entries'], N),
    ("so-unflatten", ["-c", 'reduce to_entries[] as $e ({}; setpath($e.key | split("."); $e.value))'], j({"a.b": 1, "a.c": 2, "d": 3})),
    ("so-find-paths-to-key", ["-c", '[paths(..) | select(.[-1] == "name")]'], N),
    ("so-recursive-find-values", ["-c", '[.. | .sku? // empty]'], N),
    ("so-recursive-objects-with-key", ["-c", '[.. | objects | select(has("qty")) | .sku]'], N),
    ("so-leaf-count", ["-c", '[paths(scalars)] | length'], N),
    ("so-array-contains", ["-e", '.meta.tags | index("b") != null'], N),
    ("so-in-list", ["-c", '[.[] | select(.team | IN("core", "ops")) | .name]'], PEOPLE),
    ("so-contains-string", ["-c", 'map(select(.city // "" | contains("Lis")) | .name)'], PEOPLE),
    ("so-test-regex-case", ["-c", 'map(select(.name | test("^[A-C]"; "i")) | .name)'], PEOPLE),
    ("so-split-and-take", ["-r", 'split("/") | last'], j("arn:aws:iam::123:role/deploy/ci")),
    ("so-string-interpolation-nested", ["-r", '"\\(.meta.owner.name) has \\(.items | length) items: \\([.items[].sku] | join("+"))"'], N),
    ("so-raw-string-no-quotes", ["-r", '.meta.tags[]'], N),
    ("so-join-output", ["-j", '.meta.tags[] | . + " "'], N),
    ("so-compact-each-element", ["-c", '.items[]'], N),
    ("so-array-to-lines-index", ["-r", '.meta.tags | to_entries[] | "\\(.key + 1). \\(.value)"'], N),
    ("so-length-of-each", ["-c", 'map(length)'], j(["abc", [1, 2], {"a": 1}, None, -5])),
    ("so-type-check", ["-c", 'map(type)'], j([1, "a", None, True, [], {}])),
    ("so-is-empty", ["-c", 'map(. == [] or . == {} or . == "" or . == null)'], j([[], {}, "", None, 0, [0]])),
    ("so-default-values", ["-c", '{port: 80, host: "localhost"} + .'], j({"host": "example.test"})),
    ("so-if-then-else", ["-c", 'map(if .age >= 30 then "senior" elif .age >= 25 then "mid" else "junior" end)'], PEOPLE),
    ("so-alternative-false", ["-c", '[.a // "d", .b // "d", .c // "d"]'], j({"a": False, "b": None, "c": 0})),
    ("so-try-tonumber", ["-c", 'map(tonumber? // null)'], j(["1", "x", "2.5", ""])),
    ("so-error-message", ["-c", '[.[] | try (if . < 0 then error("negative: \\(.)") else . end) catch .]'], j([1, -2, 3])),
    ("so-env-var", ["-n", "-r", '$ENV.HOME | type'], ""),
    ("so-arg-number", ["-n", "-c", "--argjson", "n", "5", '[range($n)] | map(. * .)'], ""),
    ("so-args-positional-json", ["-n", "-c", '$ARGS.positional | add', "--jsonargs", "1", "2", "3"], ""),
    ("so-arg-into-filter", ["-c", "--arg", "sku", "x1", '[.items[] | select(.sku == $sku)] | length'], N),
    ("so-slurpfile", ["-n", "-c", "--slurpfile", "d", "data.json", '$d | map(.n) | add'], None),
    ("so-rawfile", ["-n", "-c", "--rawfile", "t", "notes.txt", '$t | split("\\n") | map(select(length > 0))'], None),
    ("so-input-two-files", ["-n", "-c", 'input as $a | input as $b | $a + $b', "a.json", "b.json"], None),
    ("so-inputs-merge-many", ["-n", "-c", 'reduce inputs as $o ({}; . * $o)', "a.json", "b.json"], None),
    ("so-null-input-build", ["-n", "-c", '{name: "x", list: [range(3)], at: (0 | todate)}'], ""),
    ("so-build-from-raw-lines", ["-R", "-n", "-c", '[inputs | select(length > 0)]'], "alpha\nbeta\n\ngamma\n"),
    ("so-raw-input-json-lines", ["-R", "-c", 'fromjson? // {raw: .}'], '{"a":1}\nnot json\n[2]\n'),
    ("so-tostring-tojson", ["-c", '[tostring, tojson, (.a | tostring)]'], j({"a": [1, "x"]})),
    ("so-escape-shell", ["-r", '@sh "echo \\(.msg)"'], j({"msg": "it's $HOME; rm -rf /"})),
    ("so-urlencode-query", ["-r", 'to_entries | map("\\(.key)=\\(.value | @uri)") | join("&")'], j({"q": "a b&c", "lang": "vi", "page": 2})),
    ("so-html-escape", ["-r", '@html'], j("<a href=\"x\">Tom & Jerry's</a>")),
    ("so-base64-roundtrip", ["-r", '@base64, (@base64 | @base64d)'], j("héllo wörld")),
    ("so-json-string-embedded", ["-c", '.payload | fromjson | .items[0]'], j({"payload": "{\"items\": [{\"id\": 1}]}"})),
    ("so-embed-as-string", ["-c", '{payload: (.data | tojson)}'], j({"data": {"x": [1, 2]}})),
    ("so-limit-first-n", ["-c", '[limit(2; .[] | select(. > 1))]'], j([1, 2, 3, 4, 5])),
    ("so-first-last-nth", ["-c", '[first, last, nth(2)]'], j([10, 20, 30, 40])),
    ("so-range-step", ["-c", '[range(0; 1; 0.25)]'], "null"),
    ("so-chunk-array", ["-c", '[_nwise(2)]'], j([1, 2, 3, 4, 5])),
    ("so-chunk-with-range", ["-c", '. as $a | [range(0; length; 3) | $a[.:. + 3]]'], j([1, 2, 3, 4, 5, 6, 7])),
    ("so-zip-arrays", ["-c", '[.keys, .values] | transpose | map({(.[0]): .[1]}) | add'], j({"keys": ["a", "b", "c"], "values": [1, 2, 3]})),
    ("so-cartesian", ["-c", '[.a[] as $x | .b[] as $y | "\\($x)\\($y)"]'], j({"a": ["x", "y"], "b": [1, 2]})),
    ("so-difference-arrays", ["-c", '.a - .b'], j({"a": [1, 2, 3, 4], "b": [2, 4]})),
    ("so-intersection", ["-c", '.a as $a | .b | map(select(. as $x | $a | index($x)))'], j({"a": [1, 2, 3], "b": [3, 4, 1]})),
    ("so-dedupe-keep-order", ["-c", 'reduce .[] as $x ([]; if index([$x]) then . else . + [$x] end)'], j([3, 1, 3, 2, 1])),
    ("so-frequency", ["-c", 'group_by(.) | map({(.[0] | tostring): length}) | add'], j(["a", "b", "a", "c", "a"])),
    ("so-running-total", ["-c", '[foreach .[] as $x (0; . + $x)]'], j([1, 2, 3, 4])),
    ("so-pairwise-diff", ["-c", '[range(1; length) as $i | .[$i] - .[$i - 1]]'], j([1, 4, 9, 16])),
    ("so-max-of-objects-key", ["-r", 'to_entries | max_by(.value) | .key'], j({"a": 3, "b": 9, "c": 1})),
    ("so-sort-object-by-value", ["-c", 'to_entries | sort_by(.value) | from_entries'], j({"a": 3, "b": 9, "c": 1})),
    ("so-keys-sorted-vs-unsorted", ["-c", '[keys, keys_unsorted]'], j({"b": 1, "a": 2})),
    ("so-sort-keys-output", ["-S", "-c", "."], j({"b": {"y": 1, "x": 2}, "a": [3, {"d": 4, "c": 5}]})),
    ("so-tab-output", ["--tab", ".meta"], N),
    ("so-indent-output", ["--indent", "1", ".items[0]"], N),
    ("so-date-now-format", ["-n", "-r", '1762138800 | strftime("%A, %B %d, %Y")'], ""),
    ("so-date-add-days", ["-r", '.date | strptime("%Y-%m-%d") | mktime + 86400 * 30 | strftime("%Y-%m-%d")'], j({"date": "2026-11-03"})),
    ("so-date-diff-days", ["-c", '[.[] | strptime("%Y-%m-%d") | mktime] | (.[1] - .[0]) / 86400'], j(["2026-01-01", "2026-11-03"])),
    ("so-iso-to-epoch-ms", ["-c", 'map(sub("\\\\.[0-9]+"; "") | fromdateiso8601 * 1000)'], j(["2026-11-03T01:00:00.123Z", "2026-11-03T02:00:00Z"])),
    ("so-epoch-ms-to-iso", ["-r", '.ts / 1000 | todate'], j({"ts": 1762131600000})),
    ("so-gsub-whitespace", ["-r", 'gsub("\\\\s+"; " ") | ltrimstr(" ") | rtrimstr(" ")'], j("  many   spaces\there \n ")),
    ("so-trim-builtin", ["-c", 'map(trim)'], j(["  a ", "\tb\n", "c"])),
    ("so-capture-version", ["-c", 'capture("v(?<major>[0-9]+)\\\\.(?<minor>[0-9]+)\\\\.(?<patch>[0-9]+)") | map_values(tonumber)'], j("release v1.42.7-beta")),
    ("so-splits-regex", ["-c", '[splits(", *|;")]'], j("a, b,c;d")),
    ("so-ascii-codes", ["-c", 'explode | map(. + 1) | implode'], j("HAL")),
    ("so-pad-left", ["-r", '.[] | tostring | (" " * (5 - length)) + .'], j([1, 22, 333])),
    ("so-number-format", ["-r", '.[] | (. * 100 | round / 100 | tostring) as $s | if ($s | contains(".")) then $s else $s + ".00" end'], j([3.14159, 2, 0.1])),
    ("so-to-entries-index", ["-c", 'to_entries | map("\\(.key):\\(.value)")'], j(["a", "b"])),
    ("so-env-export", ["-r", 'to_entries[] | "export \\(.key | ascii_upcase)=\\(.value | tostring | @sh)"'], j({"db_host": "x y", "port": 5432})),
    ("so-reduce-to-max", ["-c", 'reduce .[] as $x (null; if . == null or $x > . then $x else . end)'], j([3, 9, 2])),
    ("so-label-break-first", ["-c", 'label $f | .[] | if . > 2 then ., break $f else empty end'], j([1, 2, 3, 4])),
    ("so-until-loop", ["-c", '[.[] | until(. >= 100; . * 3)]'], j([1, 5, 50])),
    ("so-recurse-tree", ["-c", '[recurse(.children[]?) | .name]'], j({"name": "root", "children": [{"name": "a", "children": [{"name": "a1"}]}, {"name": "b"}]})),
    ("so-tree-depth", ["-c", 'def depth: if (.children // []) == [] then 1 else 1 + ([.children[] | depth] | max) end; depth'], j({"name": "root", "children": [{"name": "a", "children": [{"name": "a1"}]}, {"name": "b"}]})),
    ("so-def-with-args", ["-c", 'def addvalue(f): . + [f]; map(addvalue(.[0]))'], j([[1, 2], [10, 20]])),
    ("so-getpath-safe", ["-c", '[getpath(["a", "b"]), getpath(["x", "y"])]'], j({"a": {"b": 1}})),
    ("so-paths-leaf-values", ["-c", '[leaf_paths as $p | [($p | map(tostring) | join("/")), getpath($p)]]'], j({"a": {"b": 1, "c": [2, 3]}})),
    ("so-tostream-fromstream", ["-c", 'fromstream(tostream)'], j({"a": [1, {"b": 2}]})),
    ("so-truncate-stream", ["-c", '[1 | truncate_stream([[0], 1], [[1, 0], 2], [[1, 0]], [[1]])]'], "null"),
    ("so-pick", ["-c", 'pick(.meta.owner.name, .items[0].sku)'], N),
    ("so-splits-to-objects", ["-R", "-c", 'split("=") | {(.[0]): .[1]}'], "a=1\nb=2\n"),
    ("so-ltrimstr-prefix-list", ["-c", 'map(ltrimstr("refs/heads/") | ltrimstr("refs/tags/"))'], j(["refs/heads/main", "refs/tags/v1", "HEAD"])),
    ("so-exit-status-select", ["-e", '.[] | select(.ok)'], j([{"ok": False}, {"ok": True}])),
    ("so-halt-error-validation", ["-e", 'if (.version | type) != "string" then "version must be a string\\n" | halt_error(2) else .version end'], j({"version": 3})),
    ("so-arg-with-newline", ["-n", "-c", "--arg", "m", "line1\\nline2", '$m | split("\\\\n")'], ""),
]

FILES = {
    "so-slurpfile": {"data.json": '{"n": 1}\n{"n": 2}\n{"n": 39}\n'},
    "so-rawfile": {"notes.txt": "first\nsecond\n\nthird\n"},
    "so-input-two-files": {"a.json": '{"a": 1}\n', "b.json": '{"b": 2}\n'},
    "so-inputs-merge-many": {"a.json": '{"x": {"y": 1}}\n', "b.json": '{"x": {"z": 2}}\n'},
}

if __name__ == "__main__":
    out = sys.argv[1]
    names = [c[0] for c in CASES]
    assert len(names) == len(set(names)), "a name twice"
    for name, args, stdin in CASES:
        d = os.path.join(out, name)
        os.makedirs(d, exist_ok=True)
        assert all("\n" not in a for a in args), name
        open(os.path.join(d, "args"), "w").write("".join(a + "\n" for a in args))
        open(os.path.join(d, "stdin"), "w").write(stdin or "")
        for f, text in FILES.get(name, {}).items():
            p = os.path.join(d, "files", f)
            os.makedirs(os.path.dirname(p), exist_ok=True)
            open(p, "w").write(text)
    print(len(CASES))
