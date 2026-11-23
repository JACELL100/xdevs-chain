'use client'

/**
 * The launch film. Nothing downloads until the visitor presses play (only the poster),
 * then it plays inline with sound and native controls.
 */
import { useRef, useState } from 'react'
import Mark from './Mark'
import { Play } from './icons'

const SRC = '/media/xdevs-chain-film.mp4'
const POSTER = '/media/xdevs-chain-film-poster.jpg'

export default function Film() {
  const [playing, setPlaying] = useState(false)
  const [poster, setPoster] = useState(true)
  const ref = useRef<HTMLVideoElement>(null)
  return (
    <div className="film">
      {playing ? (
        <video ref={ref} src={SRC} poster={POSTER} controls autoPlay playsInline preload="auto" aria-label="xDEVS Chain launch film" />
      ) : (
        <>
          {poster ? (
            <img src={POSTER} alt="" loading="lazy" decoding="async" width={1920} height={1080} onError={() => setPoster(false)} />
          ) : (
            <div className="fallback-mark" aria-hidden="true">
              <Mark uid="film" />
            </div>
          )}
          <div className="shade" />
          <button type="button" className="film-play" onClick={() => setPlaying(true)}>
            <span className="disc">
              <Play />
            </span>
            <span>
              Watch the film
              <small>The launch film. Plays here, with sound.</small>
            </span>
          </button>
        </>
      )}
    </div>
  )
}
