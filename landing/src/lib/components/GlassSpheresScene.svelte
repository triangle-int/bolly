<script lang="ts">
	import { onMount } from 'svelte';
	import { T, useTask, useThrelte } from '@threlte/core';
	import { HTML, Text } from '@threlte/extras';
	import * as THREE from 'three';

	const { scene, camera, renderer, size } = useThrelte();

	// ── HTML → Texture (exact copy of three-html-render approach) ──
	class HtmlRenderer {
		async update(node: HTMLElement): Promise<THREE.CanvasTexture> {
			const w = node.clientWidth;
			const h = node.clientHeight;

			const clone = node.cloneNode(true) as HTMLElement;
			// Fix styles for foreignObject context
			const fixEl = (el: HTMLElement) => {
				el.style.boxSizing = 'border-box';
				el.style.backdropFilter = 'none';
				el.style.webkitBackdropFilter = 'none';
				if (el.style.background?.includes('rgba')) el.style.background = '#0a0a14';
			};
			fixEl(clone);
			clone.querySelectorAll<HTMLElement>('*').forEach(fixEl);

			const serialized = new XMLSerializer().serializeToString(clone);
			const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${h}"><foreignObject width="${w}" height="${h}">${serialized}</foreignObject></svg>`;
			const dataUrl = 'data:image/svg+xml;charset=utf-8,' + encodeURIComponent(svg);

			const image = await new Promise<HTMLImageElement>((resolve, reject) => {
				const img = new Image();
				img.onload = () => resolve(img);
				img.onerror = reject;
				img.src = dataUrl;
			});

			const canvas = document.createElement('canvas');
			canvas.width = w;
			canvas.height = h;
			const ctx = canvas.getContext('2d')!;
			ctx.fillStyle = '#000206';
			ctx.fillRect(0, 0, w, h);
			ctx.drawImage(image, 0, 0, w, h);

			const tex = new THREE.CanvasTexture(canvas);
			tex.needsUpdate = true;
			return tex;
		}
	}

	const htmlRenderer = new HtmlRenderer();
	const backingPlanes: { mesh: THREE.Mesh; el: HTMLElement }[] = [];

	// Gradient background matching client: dark bottom → slightly lighter top
	// Client uses rgb(0.002, 0.003, 0.012) → rgb(0.003, 0.005, 0.018)
	// We approximate with a vertical gradient via a large background plane
	scene.background = new THREE.Color(0x0a0a14); // dark indigo fallback

	const CAM_Z = 14;
	const FOV = 50;
	const TOTAL_HEIGHT = 85;

	// ── Render target for refraction ──
	let fbo: THREE.WebGLRenderTarget | null = null;

	// ── Fonts ──
	const fraunces = 'https://fonts.gstatic.com/s/fraunces/v38/6NVf8FyLNQOQZAnv9ZwNjucMHVn85Ni7emAe9lKqZTnbB-gzTK0K1ChJdt9vIVYX9G37lod9sPEKsxx664UJf1hLTf7W.ttf';
	const bricolage = 'https://fonts.gstatic.com/s/bricolagegrotesque/v9/3y9U6as8bTXq_nANBjzKo3IeZx8z6up5BeSl5jBNz_19PpbJMXuECpwUxJBOm_OJWiaaD30YfKfjZZoLvRviyM0.ttf';

	// ── Colors ──
	const warm = '#c4a265';
	const text = '#e6dcc8';
	const dim = '#8a8070';
	const ghost = '#55504a';

	// ── Background gradient (matching client flat background) ──
	const bgGradientMat = new THREE.ShaderMaterial({
		vertexShader: `varying vec2 vUv; void main() { vUv = uv; gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0); }`,
		fragmentShader: `
			varying vec2 vUv;
			void main() {
				// Dark indigo gradient matching client screenshot
				vec3 bottom = vec3(0.04, 0.04, 0.08);  // dark indigo
				vec3 top = vec3(0.06, 0.06, 0.12);      // slightly lighter indigo
				vec3 col = mix(bottom, top, vUv.y);
				// Subtle purple-blue aurora band
				col += vec3(0.01, 0.008, 0.025) * exp(-pow((vUv.y - 0.5) / 0.2, 2.0));
				// Subtle warm accent near bottom
				col += vec3(0.008, 0.005, 0.002) * exp(-pow((vUv.y - 0.2) / 0.15, 2.0));
				gl_FragColor = vec4(col, 1.0);
			}
		`,
		depthWrite: false,
	});

	// ── Starfield ──
	const STAR_COUNT = 1500;
	const starPos = new Float32Array(STAR_COUNT * 3);
	const starCol = new Float32Array(STAR_COUNT * 3);
	for (let i = 0; i < STAR_COUNT; i++) {
		const theta = Math.random() * Math.PI * 2;
		const phi = Math.acos(2 * Math.random() - 1);
		const r = 15 + Math.random() * 45;
		starPos[i * 3] = r * Math.sin(phi) * Math.cos(theta);
		starPos[i * 3 + 1] = r * Math.sin(phi) * Math.sin(theta);
		starPos[i * 3 + 2] = r * Math.cos(phi);
		const w = Math.random();
		starCol[i * 3] = 0.6 + w * 0.4;
		starCol[i * 3 + 1] = 0.7 + w * 0.2;
		starCol[i * 3 + 2] = 1.0 - w * 0.3;
	}
	const starGeo = new THREE.BufferGeometry();
	starGeo.setAttribute('position', new THREE.BufferAttribute(starPos, 3));
	starGeo.setAttribute('color', new THREE.BufferAttribute(starCol, 3));
	const starMat = new THREE.PointsMaterial({
		size: 0.08, sizeAttenuation: true, vertexColors: true,
		transparent: true, opacity: 0.7, depthWrite: false,
		blending: THREE.AdditiveBlending,
	});

	// ── Glass refraction shader (matching client's blob style) ──
	const refractionMat = new THREE.ShaderMaterial({
		vertexShader: /* glsl */ `
			uniform float uTime;
			uniform float uSpeed;
			uniform float uIntensity;
			uniform float uBreathe;

			varying vec3 vWorldNormal;
			varying vec3 vViewDir;
			varying vec2 vScreenUV;

			void main() {
				// Organic blob displacement (same as client SharedScene)
				float noise = sin(position.x * 2.1 + uTime * uSpeed * 0.7)
					* cos(position.y * 1.8 + uTime * uSpeed * 0.5)
					* sin(position.z * 2.5 + uTime * uSpeed * 0.9);
				vec3 displaced = position * (uBreathe + noise * uIntensity);

				vec4 worldPos = modelMatrix * vec4(displaced, 1.0);

				// Recompute normal from displacement (approximate)
				float e = 0.01;
				float nx = sin((position.x+e)*2.1+uTime*uSpeed*0.7)*cos(position.y*1.8+uTime*uSpeed*0.5)*sin(position.z*2.5+uTime*uSpeed*0.9)
				         - sin((position.x-e)*2.1+uTime*uSpeed*0.7)*cos(position.y*1.8+uTime*uSpeed*0.5)*sin(position.z*2.5+uTime*uSpeed*0.9);
				float ny = sin(position.x*2.1+uTime*uSpeed*0.7)*cos((position.y+e)*1.8+uTime*uSpeed*0.5)*sin(position.z*2.5+uTime*uSpeed*0.9)
				         - sin(position.x*2.1+uTime*uSpeed*0.7)*cos((position.y-e)*1.8+uTime*uSpeed*0.5)*sin(position.z*2.5+uTime*uSpeed*0.9);
				float nz = sin(position.x*2.1+uTime*uSpeed*0.7)*cos(position.y*1.8+uTime*uSpeed*0.5)*sin((position.z+e)*2.5+uTime*uSpeed*0.9)
				         - sin(position.x*2.1+uTime*uSpeed*0.7)*cos(position.y*1.8+uTime*uSpeed*0.5)*sin((position.z-e)*2.5+uTime*uSpeed*0.9);
				vec3 displacedNormal = normalize(normal + vec3(nx, ny, nz) * uIntensity * 2.0);

				vWorldNormal = normalize((modelMatrix * vec4(displacedNormal, 0.0)).xyz);
				vViewDir = normalize(worldPos.xyz - cameraPosition);
				vec4 clip = projectionMatrix * viewMatrix * worldPos;
				vScreenUV = clip.xy / clip.w * 0.5 + 0.5;
				gl_Position = clip;
			}
		`,
		fragmentShader: /* glsl */ `
			uniform sampler2D uSceneTex;
			uniform float uIOR;
			uniform float uChroma;
			uniform float uFresnelPow;

			varying vec3 vWorldNormal;
			varying vec3 vViewDir;
			varying vec2 vScreenUV;

			void main() {
				vec3 N = normalize(vWorldNormal);
				vec3 V = normalize(vViewDir);

				// Snell refraction
				vec3 refr = refract(V, N, 1.0 / uIOR);
				vec2 offset = refr.xy * 0.12;

				// Chromatic aberration (dispersion)
				float r = texture2D(uSceneTex, vScreenUV + offset * (1.0 + uChroma)).r;
				float g = texture2D(uSceneTex, vScreenUV + offset).g;
				float b = texture2D(uSceneTex, vScreenUV + offset * (1.0 - uChroma)).b;
				vec3 refracted = vec3(r, g, b);

				// Fresnel
				float fresnel = pow(1.0 + dot(V, N), uFresnelPow);

				// Specular highlights (key + rim, matching client lighting)
				vec3 refl = reflect(V, N);
				float specKey = pow(max(dot(refl, normalize(vec3(3.0, 2.0, 4.0))), 0.0), 80.0) * 0.4;
				float specRim = pow(max(dot(refl, normalize(vec3(-3.0, 1.5, -2.0))), 0.0), 40.0) * 0.15;

				// Combine
				vec3 col = mix(refracted, vec3(0.05, 0.07, 0.14), fresnel * 0.2);
				col += (specKey + specRim) * vec3(0.9, 0.92, 1.0);
				col += fresnel * vec3(0.03, 0.05, 0.10) * 0.3;

				gl_FragColor = vec4(col, 1.0);
			}
		`,
		uniforms: {
			uSceneTex: { value: null },
			uIOR: { value: 1.45 },
			uChroma: { value: 0.08 },
			uFresnelPow: { value: 2.5 },
			uTime: { value: 0 },
			uSpeed: { value: 0.8 },
			uIntensity: { value: 0.08 },
			uBreathe: { value: 1.0 },
		},
	});
	const geoLarge = new THREE.IcosahedronGeometry(1, 12);
	const geoMedium = new THREE.IcosahedronGeometry(0.6, 8);
	const geoSmall = new THREE.IcosahedronGeometry(0.35, 6);

	// ── Sphere definitions scattered along scroll ──
	interface SphereConf {
		x: number; y: number; z: number;
		size: 'l' | 'm' | 's';
		phase: number; // unique offset for morphing
	}

	const sphereConfs: SphereConf[] = [
		// Hero area
		{ x: 5, y: 2, z: 2, size: 'l', phase: 0 },
		{ x: -6, y: -1, z: 1.5, size: 'm', phase: 1.2 },
		{ x: 3, y: -4, z: 1, size: 's', phase: 2.4 },
		// Features
		{ x: -7, y: -14, z: 2, size: 'm', phase: 0.8 },
		{ x: 7, y: -18, z: 1.5, size: 'l', phase: 3.1 },
		{ x: 0, y: -22, z: 1, size: 's', phase: 1.7 },
		// Demo
		{ x: 6, y: -30, z: 2, size: 'm', phase: 0.3 },
		{ x: -3, y: -35, z: 1.5, size: 's', phase: 2.8 },
		// How it works
		{ x: -6, y: -46, z: 2, size: 'l', phase: 1.5 },
		{ x: 5, y: -50, z: 1, size: 's', phase: 3.6 },
		// Pricing
		{ x: 7, y: -62, z: 2, size: 'm', phase: 0.6 },
		{ x: -5, y: -67, z: 1.5, size: 'l', phase: 2.1 },
		{ x: 2, y: -70, z: 1, size: 's', phase: 4.0 },
		// CTA / Footer
		{ x: -4, y: -78, z: 2, size: 'm', phase: 1.0 },
		{ x: 5, y: -84, z: 1.5, size: 's', phase: 3.3 },
	];

	const sphereMeshes: THREE.Mesh[] = [];
	// Smooth current positions — spheres lerp toward targets
	const sphereCurrentPos: THREE.Vector3[] = [];

	// ── Cinematic intro — blur wall → spiral away from camera ──
	let introStartTime = 0;
	const INTRO_BLUR = 2.0;      // seconds: all spheres smashed against camera = total blur
	const INTRO_SPIRAL = 4.0;    // seconds: spiral away from camera to ring
	const INTRO_TOTAL = INTRO_BLUR + INTRO_SPIRAL;
	const INTRO_RING_RADIUS = 6;
	const SPHERE_SPEED = 1.0;    // units per second after intro
	let starsRef: THREE.Points | undefined;
	let bgPlane: THREE.Mesh | undefined;

	// ── Particle trails ──
	const TRAIL_COUNT = 4; // particles per sphere
	const trailGeo = new THREE.BufferGeometry();
	const trailPositions = new Float32Array(sphereConfs.length * TRAIL_COUNT * 3);
	const trailAlphas = new Float32Array(sphereConfs.length * TRAIL_COUNT);
	trailGeo.setAttribute('position', new THREE.BufferAttribute(trailPositions, 3));
	trailGeo.setAttribute('alpha', new THREE.BufferAttribute(trailAlphas, 1));

	const trailMat = new THREE.ShaderMaterial({
		vertexShader: `
			attribute float alpha;
			varying float vAlpha;
			void main() {
				vAlpha = alpha;
				vec4 mvPos = modelViewMatrix * vec4(position, 1.0);
				gl_PointSize = (1.0 + alpha * 2.0) * (300.0 / -mvPos.z);
				gl_Position = projectionMatrix * mvPos;
			}
		`,
		fragmentShader: `
			varying float vAlpha;
			void main() {
				float d = length(gl_PointCoord - 0.5) * 2.0;
				if (d > 1.0) discard;
				float glow = exp(-d * d * 3.0) * vAlpha;
				gl_FragColor = vec4(0.4, 0.5, 0.8, glow * 0.6);
			}
		`,
		transparent: true,
		depthWrite: false,
		blending: THREE.AdditiveBlending,
	});

	// Trail history — stores last N positions per sphere
	const trailHistory: THREE.Vector3[][] = sphereConfs.map(() =>
		Array.from({ length: TRAIL_COUNT }, () => new THREE.Vector3())
	);

	// ── Features data ──
	const features = [
		{ title: 'helps you focus', desc: 'Track your tasks, break down complex goals, and stay on course.' },
		{ title: 'studies with you', desc: 'Explain concepts, quiz you, discuss what you\'re reading.' },
		{ title: 'feels your mood', desc: 'Notices when you\'re stressed, tired, or excited.' },
		{ title: 'thinks with you', desc: 'Talk through ideas, decisions, creative blocks.' },
		{ title: 'checks in on you', desc: 'Wakes up on its own. Reflects, journals, reaches out.' },
		{ title: 'completely private', desc: 'Fully encrypted and private. No one else can access.' },
	];

	// ── Steps data ──
	const steps = [
		{ num: '01', title: 'install', desc: 'Run Bolly on your own machine.' },
		{ num: '02', title: 'shape who they are', desc: 'Choose a personality or write your own.' },
		{ num: '03', title: 'just talk', desc: 'They remember everything. Their mood shifts. They grow.' },
	];

	// ── Scroll & mouse ──

	// ── Auto-capture all HTML elements after mount ──
	onMount(() => {
		// Create background gradient plane
		bgPlane = new THREE.Mesh(
			new THREE.PlaneGeometry(200, 200),
			bgGradientMat
		);
		bgPlane.position.set(0, 0, -10);
		bgPlane.renderOrder = -100;
		scene.add(bgPlane);

		introStartTime = performance.now() / 1000;

		// Anchor link handling — intercept clicks, handle hash on load
		function onAnchorClick(e: MouseEvent) {
			const a = (e.target as HTMLElement).closest('a');
			if (!a) return;
			const href = a.getAttribute('href');
			if (!href) return;
			const m = href.match(/#(\w+)$/);
			if (m && sectionMap['#' + m[1]] !== undefined) {
				e.preventDefault();
				scrollToSection('#' + m[1]);
			}
		}
		document.addEventListener('click', onAnchorClick);

		// Scroll to hash on load
		if (window.location.hash && sectionMap[window.location.hash] !== undefined) {
			setTimeout(() => scrollToSection(window.location.hash), 800);
		}

		// Create sphere meshes — they all start clustered at camera
		for (const conf of sphereConfs) {
			const geo = conf.size === 'l' ? geoLarge : conf.size === 'm' ? geoMedium : geoSmall;
			const mesh = new THREE.Mesh(geo, refractionMat);
			mesh.position.set(conf.x, conf.y, conf.z);
			mesh.userData = conf;
			scene.add(mesh);
			sphereMeshes.push(mesh);
			sphereCurrentPos.push(new THREE.Vector3(0, 0, CAM_Z - 4)); // start at camera
		}

		setTimeout(async () => {
			// ── Capture HTML elements and place backing planes at known 3D positions ──
			const canvasParent = renderer.domElement.parentElement;
			if (!canvasParent) return;

			// ── Capture by data-backing attribute ──
			const pxToUnit = 0.024;

			const backingDefs: { id: string; x: number; y: number; s: number }[] = [];
			for (let i = 0; i < 6; i++) {
				const col = i % 3, row = Math.floor(i / 3);
				backingDefs.push({ id: `feature-${i}`, x: -6 + col * 6, y: Y.features - row * 4.5, s: 0.6 });
			}
			backingDefs.push({ id: 'demo', x: -4, y: Y.demo + 1, s: 0.7 });
			for (let i = 0; i < 3; i++) backingDefs.push({ id: `step-${i}`, x: -5 + i * 5, y: Y.how - 0.5, s: 0.5 });

			let count = 0;
			for (const def of backingDefs) {
				const el = document.querySelector<HTMLElement>(`[data-backing="${def.id}"]`);
				if (!el) continue;

				try {
					const tex = await htmlRenderer.update(el);
					const pw = el.clientWidth * pxToUnit * def.s;
					const ph = el.clientHeight * pxToUnit * def.s;

					const mesh = new THREE.Mesh(
						new THREE.PlaneGeometry(pw, ph),
						new THREE.MeshBasicMaterial({ map: tex, side: THREE.DoubleSide })
					);
					mesh.position.set(def.x, def.y, 0.01);
					mesh.visible = false;
					scene.add(mesh);
					backingPlanes.push({ mesh, el });
					count++;
				} catch {}
			}


		}, 2500);
	});

	let scrollY = 0;
	let scrollProgress = 0;
	let smoothCamY = 0;
	let mouseX = 0, mouseY = 0, targetMX = 0, targetMY = 0;

	if (typeof window !== 'undefined') {
		window.addEventListener('scroll', () => { scrollY = window.scrollY; }, { passive: true });
		window.addEventListener('mousemove', (e) => {
			targetMX = (e.clientX / window.innerWidth - 0.5) * 2;
			targetMY = (e.clientY / window.innerHeight - 0.5) * 2;
		}, { passive: true });
	}

	useTask(() => {
		const t = performance.now() / 1000;

		// Resize FBO
		const w = Math.round(size.current.width * Math.min(devicePixelRatio, 2));
		const h = Math.round(size.current.height * Math.min(devicePixelRatio, 2));
		if (w > 0 && h > 0) {
			if (!fbo || fbo.width !== w || fbo.height !== h) {
				fbo?.dispose();
				fbo = new THREE.WebGLRenderTarget(w, h);
				refractionMat.uniforms.uSceneTex.value = fbo.texture;
			}
		}

		const maxScroll = typeof document !== 'undefined'
			? Math.max(1, document.body.scrollHeight - window.innerHeight) : 1;
		scrollProgress = scrollY / maxScroll;
		const targetCamY = -scrollProgress * TOTAL_HEIGHT;
		smoothCamY += (targetCamY - smoothCamY) * 0.1;

		mouseX += (targetMX - mouseX) * 0.04;
		mouseY += (targetMY - mouseY) * 0.04;

		camera.current.position.set(mouseX * 0.3, smoothCamY - mouseY * 0.15, CAM_Z);
		camera.current.lookAt(0, smoothCamY, 0);

		const cy = smoothCamY;

		// Update shader uniforms — morphing via time, breathing
		const breathe = Math.sin(t * 0.8) * 0.04;
		refractionMat.uniforms.uTime.value = t;
		refractionMat.uniforms.uBreathe.value = 1.0 + breathe * 0.5;

		// ── Intro: blur wall → spiral away from camera → drift to positions ──
		const elapsed = t - introStartTime;
		const introActive = elapsed < INTRO_TOTAL;

		const totalSpheres = sphereMeshes.length;

		for (let si = 0; si < totalSpheres; si++) {
			const mesh = sphereMeshes[si];
			const conf = mesh.userData as SphereConf;
			const p = conf.phase;
			const cur = sphereCurrentPos[si];

			// Target position (with drift)
			const orbitAngle = scrollProgress * Math.PI * 4 + p;
			const orbitRadius = Math.abs(conf.x);
			const driftX = Math.sin(t * 0.2 + p) * 1.5 + Math.sin(t * 0.5 + p * 2.3) * 0.5;
			const driftY = Math.sin(t * 0.15 + p * 1.7) * 0.8 + Math.cos(t * 0.35 + p * 0.9) * 0.4;
			const driftZ = Math.sin(t * 0.25 + p * 3.1) * 1.0;
			const targetX = orbitRadius * Math.cos(orbitAngle) + driftX + mouseX * 0.2;
			const targetY = conf.y + driftY;
			const targetZ = orbitRadius * 0.4 * Math.sin(orbitAngle) + driftZ;

			if (introActive) {
				if (elapsed < INTRO_BLUR) {
					// Phase 1: BLUR WALL — all spheres packed right in front of camera
					const jitter = Math.sin(t * 3 + p * 5) * 0.3;
					cur.x = Math.sin(p * 5) * 1.2 + jitter;
					cur.y = Math.cos(p * 3) * 0.8 + Math.cos(t * 2 + p * 3) * 0.2;
					cur.z = CAM_Z - 4 + Math.sin(p * 7) * 0.3;
					mesh.scale.setScalar(2.0 + Math.sin(t * 1.5 + p) * 0.2);
				} else {
					// Phase 2: SPIRAL AWAY — peel off from camera, spiral to ring
					const spiralElapsed = elapsed - INTRO_BLUR;
					const spiralProgress = Math.min(1, spiralElapsed / INTRO_SPIRAL);
					const eased = 1 - Math.pow(1 - spiralProgress, 3); // ease-out

					const anglePerSphere = (Math.PI * 2) / totalSpheres;
					const ringAngle = si * anglePerSphere + eased * Math.PI * 2.5;
					const radius = eased * INTRO_RING_RADIUS;

					// From camera position to ring position
					const fromX = Math.sin(p * 5) * 1.2;
					const fromY = Math.cos(p * 3) * 0.8;
					const fromZ = CAM_Z - 4;

					const toX = radius * Math.cos(ringAngle);
					const toY = radius * 0.3 * Math.sin(ringAngle);
					const toZ = radius * 0.25 * Math.sin(ringAngle * 0.7);

					cur.x = THREE.MathUtils.lerp(fromX, toX, eased);
					cur.y = THREE.MathUtils.lerp(fromY, toY, eased);
					cur.z = THREE.MathUtils.lerp(fromZ, toZ, eased);
					mesh.scale.setScalar(THREE.MathUtils.lerp(2.0, 1, eased));
				}
				mesh.position.copy(cur);
			} else {
				// Post-intro: move toward target at fixed speed
				const dx = targetX - cur.x;
				const dy = targetY - cur.y;
				const dz = targetZ - cur.z;
				const dist = Math.sqrt(dx * dx + dy * dy + dz * dz);

				if (dist > 0.05) {
					const step = Math.min(SPHERE_SPEED / 60, dist); // 1 unit/sec at 60fps
					const ratio = step / dist;
					cur.x += dx * ratio;
					cur.y += dy * ratio;
					cur.z += dz * ratio;
				} else {
					cur.set(targetX, targetY, targetZ);
				}

				mesh.position.copy(cur);
				mesh.scale.setScalar(1 + Math.sin(t * 0.8 + p) * 0.04);
			}

			// Update particle trail
			const trail = trailHistory[si];
			for (let ti = TRAIL_COUNT - 1; ti > 0; ti--) trail[ti].copy(trail[ti - 1]);
			trail[0].copy(mesh.position);
			for (let ti = 0; ti < TRAIL_COUNT; ti++) {
				const idx = (si * TRAIL_COUNT + ti) * 3;
				trailPositions[idx] = trail[ti].x;
				trailPositions[idx + 1] = trail[ti].y;
				trailPositions[idx + 2] = trail[ti].z;
				trailAlphas[si * TRAIL_COUNT + ti] = (1 - ti / TRAIL_COUNT) * 0.4;
			}
		}
		trailGeo.attributes.position.needsUpdate = true;
		trailGeo.attributes.alpha.needsUpdate = true;

		if (starsRef) starsRef.rotation.y = t * 0.005 + mouseX * 0.02;

		if (bgPlane) bgPlane.position.y = smoothCamY;

		// ── Two-pass render ──
		if (fbo && sphereMeshes.length > 0) {
			// Pass 1: hide spheres, show backing planes → render to FBO
			for (const m of sphereMeshes) m.visible = false;
			for (const bp of backingPlanes) bp.mesh.visible = true;

			renderer.setRenderTarget(fbo);
			renderer.clear();
			renderer.render(scene, camera.current);

			// Pass 2: show spheres, hide backing planes → render with bloom
			for (const m of sphereMeshes) m.visible = true;
			for (const bp of backingPlanes) bp.mesh.visible = false;

			renderer.setRenderTarget(null);
			renderer.clear();
			renderer.render(scene, camera.current);
		}
	});

	// ── Section Y positions ──
	const Y = {
		hero: 0,
		features: -16,
		demo: -32,
		how: -48,
		pricing: -64,
		cta: -78,
		footer: -88,
	};

	const sectionMap: Record<string, number> = {
		'#hero': Y.hero,
		'#features': Y.features,
		'#demo': Y.demo,
		'#how': Y.how,
		'#pricing': Y.pricing,
		'#cta': Y.cta,
		'#footer': Y.footer,
	};

	function scrollToSection(hash: string) {
		if (typeof document === 'undefined') return;
		const sectionY = sectionMap[hash];
		if (sectionY === undefined) return;
		const progress = Math.abs(sectionY) / TOTAL_HEIGHT;
		const maxScroll = document.body.scrollHeight - window.innerHeight;
		window.scrollTo({ top: progress * maxScroll, behavior: 'smooth' });
		history.replaceState(null, '', hash);
	}

</script>

<T.PerspectiveCamera makeDefault position={[0, 0, CAM_Z]} fov={FOV} near={0.1} far={100} />

<!-- Lighting (matching client SharedScene) -->
<T.AmbientLight color={0x334477} intensity={0.3} />
<T.DirectionalLight color={0xffffff} intensity={2.0} position={[3, 2, 4]} />
<T.PointLight color={0x8899cc} intensity={1.0} position={[-3, 1.5, -2]} />
<T.PointLight color={0x6677aa} intensity={0.4} position={[0, -3, 1]} />

<T.Points bind:ref={starsRef} geometry={starGeo} material={starMat} />

<!-- Particle trails behind spheres -->
<T.Points geometry={trailGeo} material={trailMat} />

<!-- ═══════════════════════════════════════
     HERO
     ═══════════════════════════════════════ -->

<HTML transform pointerEvents="none" position={[0, Y.hero + 4, 0]} scale={0.7}>
	<div style="display:inline-flex;align-items:center;gap:0.5rem;padding:0.375rem 1rem;border-radius:2rem;background:oklch(1 0 0 / 5%);backdrop-filter:blur(20px) saturate(160%) brightness(1.06);border:1px solid oklch(1 0 0 / 10%);font-family:'Bricolage Grotesque',sans-serif;font-size:0.85rem;color:rgba(230,220,200,0.5);letter-spacing:0.05em;">
		<span style="width:5px;height:5px;border-radius:50%;background:#c4a265;box-shadow:0 0 8px rgba(196,162,101,0.4);"></span>
		now in beta
	</div>
</HTML>

<Text text="a friend that helps you" font={fraunces} fontSize={1.5} color={text} anchorX="center" anchorY="middle" position={[0, Y.hero + 2, 0]} textAlign="center" />
<Text text="think, work & feel" font={fraunces} fontSize={1.5} color={warm} anchorX="center" anchorY="middle" position={[0, Y.hero + 0.2, 0]} textAlign="center" />
<Text text="Not a chatbot. A presence that remembers your goals, notices your mood, helps you study, and checks in when you've been quiet too long." font={bricolage} fontSize={0.32} color={dim} anchorX="center" anchorY="top" position={[0, Y.hero - 1, 0]} maxWidth={11} textAlign="center" lineHeight={1.5} />

<HTML transform pointerEvents="auto" position={[0, Y.hero - 3.5, 0]} scale={0.7}>
	<a href="/docs" style="display:inline-flex;align-items:center;gap:0.5rem;padding:0.75rem 1.75rem;border-radius:2rem;background:oklch(1 0 0 / 5%);backdrop-filter:blur(20px) saturate(160%) brightness(1.06);border:1px solid oklch(1 0 0 / 10%);color:#c4a265;font-family:'Bricolage Grotesque',sans-serif;font-size:0.875rem;font-weight:500;text-decoration:none;">Meet yours →</a>
</HTML>

<!-- ═══════════════════════════════════════
     FEATURES
     ═══════════════════════════════════════ -->

<Text text="What it does" font={bricolage} fontSize={0.22} color={warm} anchorX="left" anchorY="middle" position={[-7, Y.features + 5, 0]} letterSpacing={0.15} />
<Text text="not another chatbot" font={fraunces} fontSize={1.0} color={text} anchorX="left" anchorY="middle" position={[-7, Y.features + 3.5, 0]} />
<Text text="Bolly is a friend that remembers, feels, and grows — one that actually helps you get through your day." font={bricolage} fontSize={0.25} color={dim} anchorX="left" anchorY="top" position={[-7, Y.features + 2.5, 0]} maxWidth={8} lineHeight={1.5} />

{#each features as f, i}
	{@const col = i % 3}
	{@const row = Math.floor(i / 3)}
	<HTML transform occlude="blending" pointerEvents="none" position={[-6 + col * 6, Y.features - row * 4.5, 0]} scale={0.6}>
		<div data-backing="feature-{i}" style="width:380px;height:160px;padding:2rem;border-radius:0;background:oklch(1 0 0 / 5%);border:1px solid oklch(1 0 0 / 10%);backdrop-filter:blur(12px);display:flex;flex-direction:column;justify-content:center;">
			<h3 style="font-family:'Fraunces',serif;font-style:italic;font-size:1.25rem;color:#e6dcc8;margin:0 0 0.75rem;">{f.title}</h3>
			<p style="font-family:'Bricolage Grotesque',sans-serif;font-size:0.9rem;color:#8a8070;line-height:1.6;margin:0;">{f.desc}</p>
		</div>
	</HTML>
{/each}

<!-- ═══════════════════════════════════════
     DEMO
     ═══════════════════════════════════════ -->

<Text text="How it feels" font={bricolage} fontSize={0.22} color={warm} anchorX="left" anchorY="middle" position={[2, Y.demo + 5, 0]} letterSpacing={0.15} />
<Text text="someone in your corner." font={fraunces} fontSize={0.85} color={text} anchorX="left" anchorY="middle" position={[2, Y.demo + 3.5, 0]} />
<Text text="not a search engine." font={fraunces} fontSize={0.85} color={dim} anchorX="left" anchorY="middle" position={[2, Y.demo + 2.3, 0]} />
<Text text="It doesn't just answer questions — it notices when you're overwhelmed and breaks things down. It remembers what you've been studying and what trips you up." font={bricolage} fontSize={0.24} color={dim} anchorX="left" anchorY="top" position={[2, Y.demo + 1.2, 0]} maxWidth={7} lineHeight={1.5} />

<!-- Demo video/chat placeholder — replace src with your video -->
<HTML transform occlude="blending" pointerEvents="none" position={[-4, Y.demo + 1, 0]} scale={0.7}>
	<div data-backing="demo" style="width:500px;height:380px;background:oklch(0.04 0.015 260);border:1px solid oklch(1 0 0 / 8%);overflow:hidden;display:flex;flex-direction:column;">
		<div style="display:flex;align-items:center;gap:0.5rem;padding:0.75rem 1rem;border-bottom:1px solid oklch(1 0 0 / 5%);">
			<div style="width:28px;height:28px;border-radius:50%;background:oklch(0.78 0.12 75 / 10%);border:1px solid oklch(0.78 0.12 75 / 15%);display:flex;align-items:center;justify-content:center;font-family:'Fraunces',serif;font-style:italic;font-size:0.8rem;color:oklch(0.78 0.12 75 / 60%);">b</div>
			<div><span style="font-size:0.85rem;color:oklch(0.90 0.02 75);">bolly</span><br/><span style="font-size:0.75rem;color:oklch(0.78 0.12 75 / 40%);font-style:italic;">feeling curious</span></div>
		</div>
		<div style="padding:1.25rem;display:flex;flex-direction:column;gap:0.6rem;font-family:'Bricolage Grotesque',sans-serif;font-size:0.85rem;flex:1;">
			<div style="align-self:flex-start;color:oklch(0.72 0.025 220 / 60%);padding:0.55rem 0.9rem;background:linear-gradient(160deg,oklch(1 0 0 / 6%),oklch(0.5 0.02 220 / 8%),oklch(1 0 0 / 3%));border:1px solid oklch(1 0 0 / 8%);border-top-color:oklch(1 0 0 / 15%);max-width:65%;">i have an exam on thursday and i haven't started studying. kind of freaking out</div>
			<div style="align-self:flex-end;color:oklch(0.9 0.025 75 / 92%);padding:0.7rem 1rem;background:linear-gradient(145deg,oklch(1 0 0 / 7%),oklch(1 0 0 / 4%),oklch(0.5 0.03 200 / 10%));border:1px solid oklch(1 0 0 / 10%);border-top-color:oklch(1 0 0 / 20%);max-width:80%;">okay let's not panic. what's the subject and what topics does it cover?</div>
			<div style="align-self:flex-start;color:oklch(0.72 0.025 220 / 60%);padding:0.55rem 0.9rem;background:linear-gradient(160deg,oklch(1 0 0 / 6%),oklch(0.5 0.02 220 / 8%),oklch(1 0 0 / 3%));border:1px solid oklch(1 0 0 / 8%);border-top-color:oklch(1 0 0 / 15%);max-width:65%;">organic chemistry</div>
			<div style="align-self:flex-end;color:oklch(0.9 0.025 75 / 92%);padding:0.7rem 1rem;background:linear-gradient(145deg,oklch(1 0 0 / 7%),oklch(1 0 0 / 4%),oklch(0.5 0.03 200 / 10%));border:1px solid oklch(1 0 0 / 10%);border-top-color:oklch(1 0 0 / 20%);max-width:80%;">three days is enough if we're smart about it. want to start with the easier topics or the hardest?</div>
		</div>
	</div>
</HTML>

<!-- ═══════════════════════════════════════
     HOW IT WORKS
     ═══════════════════════════════════════ -->

<Text text="How it works" font={bricolage} fontSize={0.22} color={warm} anchorX="center" anchorY="middle" position={[0, Y.how + 5, 0]} letterSpacing={0.15} />
<Text text="three minutes to someone" font={fraunces} fontSize={0.95} color={text} anchorX="center" anchorY="middle" position={[0, Y.how + 3.5, 0]} textAlign="center" />
<Text text="who gets you" font={fraunces} fontSize={0.95} color={text} anchorX="center" anchorY="middle" position={[0, Y.how + 2.3, 0]} textAlign="center" />

{#each steps as step, i}
	<HTML transform occlude="blending" pointerEvents="none" position={[-5 + i * 5, Y.how - 0.5, 0]} scale={0.5}>
		<div data-backing="step-{i}" style="width:300px;padding:1.5rem;border-radius:0;background:oklch(1 0 0 / 5%);border:1px solid oklch(1 0 0 / 10%);backdrop-filter:blur(12px);">
			<div style="font-family:'Fraunces',serif;font-style:italic;font-size:2.5rem;color:oklch(1 0 0 / 5%);line-height:1;margin-bottom:0.75rem;">{step.num}</div>
			<h3 style="font-family:'Fraunces',serif;font-style:italic;font-size:1.15rem;color:#e6dcc8;margin:0 0 0.5rem;">{step.title}</h3>
			<p style="font-family:'Bricolage Grotesque',sans-serif;font-size:0.8rem;color:#8a8070;line-height:1.5;margin:0;">{step.desc}</p>
		</div>
	</HTML>
{/each}

<!-- ═══════════════════════════════════════
     CTA
     ═══════════════════════════════════════ -->

<Text text="someone is ready to" font={fraunces} fontSize={1.1} color={text} anchorX="center" anchorY="middle" position={[0, Y.cta + 2.5, 0]} textAlign="center" />
<Text text="be in your corner" font={fraunces} fontSize={1.1} color={warm} anchorX="center" anchorY="middle" position={[0, Y.cta + 1, 0]} textAlign="center" />
<Text text="They'll remember what matters to you, notice how you're feeling, and be there at 3am when no one else is." font={bricolage} fontSize={0.26} color={dim} anchorX="center" anchorY="top" position={[0, Y.cta - 0.2, 0]} maxWidth={10} textAlign="center" lineHeight={1.5} />

<HTML transform pointerEvents="auto" position={[0, Y.cta - 2, 0]} scale={0.7}>
	<a href="/docs" style="display:inline-flex;align-items:center;gap:0.5rem;padding:0.75rem 1.75rem;border-radius:2rem;background:oklch(1 0 0 / 5%);backdrop-filter:blur(20px) saturate(160%) brightness(1.06);border:1px solid oklch(1 0 0 / 10%);color:#c4a265;font-family:'Bricolage Grotesque',sans-serif;font-size:0.875rem;font-weight:500;text-decoration:none;">Meet yours →</a>
</HTML>

<!-- ═══════════════════════════════════════
     FOOTER
     ═══════════════════════════════════════ -->

<Text text="© 2026 Bolly · Triangle Interactive" font={bricolage} fontSize={0.18} color={ghost} anchorX="center" anchorY="middle" position={[0, Y.footer, 0]} textAlign="center" />

<HTML transform pointerEvents="auto" position={[0, Y.footer - 1, 0]} scale={0.5}>
	<div style="display:flex;gap:1.5rem;font-family:'Bricolage Grotesque',sans-serif;font-size:0.75rem;">
		<a href="/privacy" style="color:#55504a;text-decoration:none;">Privacy</a>
		<a href="/terms" style="color:#55504a;text-decoration:none;">Terms</a>
		<a href="/docs" style="color:#55504a;text-decoration:none;">Docs</a>
	</div>
</HTML>

<!-- ═══ SPHERES ═══ -->
<!-- Glass spheres created programmatically in onMount -->
{#each sphereMeshes as mesh}
	<T is={mesh} />
{/each}
