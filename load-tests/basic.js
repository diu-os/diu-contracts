// Load Test: Baseline — physics.diu-os.org
// QAR Q-01: p95 < 3s при 50 одновременных пользователях
// Запуск: k6 run load-tests/basic.js
// Результаты: load-tests/results/YYYY-MM-DD-basic.json (git-ignored)
// Grant evidence: "Platform sustains 50 concurrent users, p95 < 3s"

import http from 'k6/http';
import { check, sleep } from 'k6';

export const options = {
  stages: [
    { duration: '1m', target: 20 },  // ramp-up
    { duration: '3m', target: 50 },  // sustained load
    { duration: '1m', target: 0 },   // ramp-down
  ],
  thresholds: {
    http_req_duration: ['p(95)<3000'],  // Q-01: первый кадр < 3s
    http_req_failed: ['rate<0.05'],
  },
};

export default function () {
  const res = http.get('https://physics.diu-os.org');
  check(res, {
    'status 200': (r) => r.status === 200,
    'loads in 3s': (r) => r.timings.duration < 3000,
  });
  sleep(1);
}
